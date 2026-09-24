# admin-auth Specification

## Purpose
How admins and maintainers prove who they are: accounts, login with password and a second
factor, sessions and CSRF protection, lockout, TOTP and recovery codes, re-authentication
and password reset (design §8 Login, Lockout, TOTP and recovery, Sessions and CSRF, Token
flows).

## Requirements

### Requirement: Accounts

Each account SHALL have a unique email address, a role (`admin` or `maintainer`), a
disabled flag and an argon2id password hash, absent until a password is set; an account
without one cannot log in. At most one account SHALL have the role `admin`; the database
itself SHALL refuse a second. Email addresses SHALL be stored and looked up with
surrounding ASCII whitespace removed and ASCII letters lowercased, and SHALL follow the
design's email rule (at most 254 characters, exactly one `@`, no whitespace or control
characters). Passwords SHALL be 12–1024 Unicode scalar values with no composition rules.
Password hashes SHALL use argon2id with 19 MiB memory, 2 iterations and parallelism 1.

#### Scenario: Attacker holds a database copy
- **WHEN** an attacker obtains a backup of an instance whose admin set the password `correct horse battery staple` and enrolled TOTP
- **THEN** the backup holds that password only as an argon2id PHC string with those parameters, and no TOTP seed, session, device, reset or recovery value in any form other than SHA-256

#### Scenario: Second admin refused
- **WHEN** any code path inserts or updates an account so that two accounts have the role `admin`
- **THEN** the database refuses the change

### Requirement: Login

`GET /admin/login` on the main host SHALL render one form with email, password and an
optional code field; `POST /admin/login` SHALL sign in only when every factor passes:

- the address names an account that has a password, is not disabled and is not locked
  for this request (see "Lockout");
- the password matches;
- if the account has TOTP enrolled, the code is a current TOTP code or an unused recovery
  code (see "Two-factor authentication").

On success Kohaku SHALL create a new session, set the session and device cookies and
answer `303` to `/admin`, never to a location taken from the request. Every failure (unknown
address, no password, disabled, locked, wrong password, missing or wrong code) SHALL get the
byte-identical response: status 200, the login form with one fixed error message and no
submitted value. Before a request is answered, argon2id SHALL run exactly once for it,
against the account's hash or a fixed dummy hash, so every failure costs the same work.

An account required to use two-factor authentication (in this change: the `admin` role)
that has none enrolled SHALL NOT get a session: after an otherwise correct password Kohaku
SHALL answer with a page saying two-factor login is required and to ask the admin for a
setup link. That answer SHALL NOT count as a failure or reset any counter.

A signed-in user requesting `GET /admin/login` SHALL get `303` to `/admin`.

#### Scenario: Attacker probes accounts
- **WHEN** an attacker posts logins for an unknown address, a disabled account, an account without a password, a locked account, and a real account with a wrong password
- **THEN** all five responses have the same status and body, none sets a cookie, and each ran argon2id once

#### Scenario: Attacker has the password but not the code
- **WHEN** an attacker posts the admin's correct password with no code, a wrong code, a code of the previous-but-one step or a used recovery code
- **THEN** each gets the generic failure, counts as a failure for the account, and no session is created

#### Scenario: No redirect target from the request
- **WHEN** a login succeeds with `?next=https://evil.example` in the URL and a `next` form field
- **THEN** the response is `303` with `Location: /admin`

### Requirement: Password hashing is bounded

At most 2 argon2id computations SHALL run at once. One permit SHALL be reserved for login
requests carrying a valid device cookie for the submitted account (checked before
hashing). A request that gets no permit within 2 s SHALL be answered 503 and SHALL NOT
count as a failure.

#### Scenario: Login flood cannot starve the owner
- **WHEN** an attacker keeps 50 login requests without a device cookie in flight, and the owner then logs in from a browser with a valid device cookie
- **THEN** at most one attacker request hashes at a time, the others wait at most 2 s and get 503, and the owner's login hashes without waiting for them

### Requirement: Lockout

Lockout state SHALL be stored per account in the database and changed only by conditional
writes on the write connection:

- A failed login without a valid device cookie for the account SHALL count toward that
  account's lock; the 10th such failure SHALL lock the account for 15 minutes, and the
  count SHALL start again from 0 when the lock ends. While locked, attempts without a valid
  device cookie SHALL fail generically without counting and without extending the lock.
- A failed login with a valid device cookie SHALL count only in a separate per-account
  window of 1 hour from its first failure; after 20 failures in a window, further attempts
  with a device cookie SHALL fail generically until the window ends. The lock above SHALL
  NOT apply to them.
- A successful login SHALL reset both counts; it SHALL NOT end a running lock.
- When a lock starts, Kohaku SHALL queue a security mail to the account unless it queued
  one within the last 24 hours. The mail SHALL state that the account was locked for 15
  minutes after 10 failed logins from unknown browsers, and contain no address, token or
  user-supplied text.
- No response SHALL say that an account is locked.

A valid device cookie is a `__Host-kohaku_device` cookie whose value's SHA-256 is stored
for that account. Kohaku SHALL set it on every successful login that did not already carry
one for the account (`Secure; HttpOnly; SameSite=Strict; Path=/`, 1-year lifetime, 256
random bits), keep at most 10 per account (the oldest removed first) and delete all of an
account's device cookies when its password changes, is reset, or its two-factor
authentication is re-enrolled or disabled.

#### Scenario: Stranger cannot lock the owner out
- **WHEN** an attacker without a device cookie sends 10 wrong passwords for the admin, then the admin logs in from a browser with a device cookie with the right password and code
- **THEN** the 10th failure locks the account and queues one lockout mail, the attacker's 11th attempt with the right password fails generically, and the admin's login succeeds

#### Scenario: Lock ends and restarts cleanly
- **WHEN** a locked account's 15 minutes pass and an attacker without a device cookie fails 9 more times
- **THEN** the account is not locked; a 10th failure locks it again, and no second lockout mail is queued within 24 hours of the first

#### Scenario: Stolen device cookie is bounded
- **WHEN** an attacker holding the admin's device cookie sends 25 wrong passwords within one hour
- **THEN** the 21st to 25th fail generically even with the right password, and an hour after the first failure a correct login succeeds

#### Scenario: Concurrent failures cannot overshoot
- **WHEN** an attacker sends 30 wrong passwords in parallel without a device cookie
- **THEN** the account is locked after exactly 10 counted failures and exactly one lockout mail is queued

### Requirement: Sessions

A session SHALL be a 256-bit random token in the `__Host-kohaku_session` cookie
(`Secure; HttpOnly; SameSite=Strict; Path=/`, no `Max-Age`, main host only); the
database SHALL store only its SHA-256. A session SHALL be valid only while its account
exists, has a password and is not disabled, less than 12 hours after its last recorded
use and less than 7 days after its creation. Kohaku SHALL record use at most once per 5
minutes per session. A login SHALL always create a new session, never reuse the token the
request presented. Changing or resetting the password SHALL end every session of the
account; enabling two-factor authentication SHALL end every other session of the account.

Without a valid session, every `/admin` path except the login, logout and reset routes
SHALL answer `GET` and `HEAD` with `303` to `/admin/login` and other methods with 403, the
same for existing and nonexistent paths. With a valid session an unknown `/admin` path
SHALL get 404.

`POST /admin/logout` with a valid CSRF token SHALL delete the session and expire the
cookie, then answer `303` to `/admin/login`.

#### Scenario: Attacker fixes or replays a session
- **WHEN** an attacker plants a known `__Host-kohaku_session` value in the victim's browser before the victim logs in, and later replays a session token taken from a backup
- **THEN** the login issues a different token, the planted value is not a session, and the backup holds only a SHA-256, not a usable token

#### Scenario: Idle and absolute expiry
- **WHEN** a session is unused for 12 hours and 5 minutes, and another is used every hour for 7 days
- **THEN** both are rejected with `303` to `/admin/login` from then on

#### Scenario: Attacker enumerates admin paths
- **WHEN** an attacker without a session requests `GET /admin`, `/admin/account`, `/admin/does-not-exist` and `POST /admin/account/password`
- **THEN** the GETs get identical `303` responses to `/admin/login`, and the POST gets 403

### Requirement: CSRF tokens

Every session SHALL have its own 256-bit random CSRF token. Every state-changing request
made with a session SHALL carry it in the form field `csrf`, compared in constant time; a
missing or different token SHALL get 403 with no state changed, in addition to the
Fetch-Metadata and Origin check (http-security). Pages SHALL embed the token only in forms
of the main host.

#### Scenario: Attacker forges a same-site form post
- **WHEN** a request with the victim's session cookie posts to `/admin/account/password` without `csrf`, with another session's token, or with a token differing in one character
- **THEN** each gets 403 and nothing changes

### Requirement: Two-factor authentication

Kohaku SHALL implement TOTP as RFC 6238 with HMAC-SHA1, 6 digits, a 30-second step from
the Unix epoch, accepting the current step and one step either side. The seed SHALL be the
first 20 bytes of the value derived from the instance secret with label `kohaku/totp`, the
account id and the account's random 16-byte enrollment nonce, and SHALL NOT be stored. A
code SHALL be accepted only if its step is later than the account's last accepted step,
decided by one conditional write that records the new step.

Enrollment SHALL produce 10 recovery codes of 16 base32 characters (80 random bits each),
shown once, stored as SHA-256 only. A recovery code SHALL be accepted in the code field
case-insensitively, with or without the hyphens it is displayed with, and consumed by one
conditional delete. Enrolling again or regenerating codes SHALL replace all of them.

The account page SHALL let a signed-in user enroll: Kohaku draws a new nonce and shows a
QR code of the key URI, plus the base32 seed as text for manual entry, and enrolls only
after a correct code for that seed is typed back. The key URI SHALL follow the
`otpauth://totp/` format authenticator apps such as FreeOTP+ and Google Authenticator scan:
label `Kohaku (<main host>):<email>`, parameters `secret` (base32, no padding), `issuer`
(`Kohaku (<main host>)`), `algorithm=SHA1`, `digits=6` and `period=30`, label and issuer
percent-encoded. The QR code SHALL be drawn into the page as inline SVG (dark modules on an
explicit light background with a 4-module quiet zone, error correction level M), within the
unchanged Content Security Policy: no `data:` URL, no script and no separate image request. Enrolling SHALL end the account's other sessions.
Disabling SHALL be refused for accounts required to use two-factor authentication.

#### Scenario: Replayed code
- **WHEN** an attacker who watched the admin type a code submits it in a second login during the same 30 seconds
- **THEN** the second login fails generically even with the right password

#### Scenario: Codes race in parallel
- **WHEN** two logins with the right password and the same valid code, or the same recovery code, arrive at once
- **THEN** exactly one succeeds

#### Scenario: Scanning the enrollment QR code
- **WHEN** a user starts enrollment and the page's QR code is decoded
- **THEN** it yields exactly the key URI with the shown seed, a code FreeOTP+ computes from it confirms enrollment, the page loads under the release CSP without a violation, and the seed appears on no other page

#### Scenario: Seed is not in the data directory
- **WHEN** an attacker holding the database, its WAL and a backup, but not `KOHAKU_SECRET`, tries to compute an enrolled account's codes
- **THEN** no stored value is the seed or its base32 text, and different nonces or account ids give different seeds

### Requirement: Re-authentication for sensitive account changes

Changing the password, starting TOTP (re-)enrollment, disabling TOTP and regenerating
recovery codes SHALL require the current password and, if TOTP is enrolled, a current code
or unused recovery code in the same request. Each such attempt SHALL draw from the `login`
rate-limit class, use a hashing permit and count failures toward the account's lockout like
a login. A password change SHALL end every session of the account (the user logs in again),
delete its device cookies and queue a security mail to the account saying its password
changed.

#### Scenario: Stolen session cannot take over the account
- **WHEN** an attacker with a victim's session cookie tries to change the password, disable TOTP or regenerate recovery codes without the current password, or with it but without a code on an enrolled account
- **THEN** each fails without change, counts as a failure for the account, and the victim's password, TOTP and recovery codes are unchanged

### Requirement: Password reset

Password reset by mail SHALL exist only for maintainers:

- `GET /admin/reset` renders a form for an email address; `POST /admin/reset` takes a
  `reset` rate-limit token, a per-address token and the public mail budget
  (request-limits), then always answers with the same page saying that a link was sent if
  an active maintainer account exists for the address, valid for one hour. For an enabled
  maintainer with a password it SHALL create a reset token valid 1 hour, invalidate the
  account's other unused reset tokens and queue a token-bearing mail with the link;
  otherwise (unknown, disabled, `admin`, no password) it SHALL queue a placeholder row
  (mail-outbox) instead, doing the same database work.
- A reset link is `/admin/reset/{token}` on the main host (host-routing's URL builder),
  the token 256 random bits in base64url without padding. `GET` SHALL only render a new
  password form for a valid token, or the same "link invalid or expired" page for any other
  value, without changing anything. `POST` SHALL take a `login` rate-limit token, validate
  the password, then consume the token in one conditional write (unused, unexpired, purpose
  reset) and only then set the password.
- Using a reset token SHALL end every session of the account, invalidate its other reset
  tokens, delete its device cookies, queue the password-changed security mail and NOT
  create a session; it SHALL NOT change the account's TOTP enrollment, recovery codes or
  lockout state.

#### Scenario: Attacker probes accounts through reset
- **WHEN** an attacker requests resets for an unknown address, the admin's address, a disabled maintainer and an active maintainer
- **THEN** the four responses are identical, exactly one queued row carries a token, and the other three are placeholder rows the worker deletes unsent

#### Scenario: Attacker with the victim's mailbox
- **WHEN** an attacker who can read a maintainer's mail uses a reset link, then tries to log in with the new password on an account with TOTP enrolled
- **THEN** the password changes, no session is created, and the login fails generically without a code

#### Scenario: Reset link replayed or raced
- **WHEN** a reset link is posted twice in parallel, posted again after use, posted after 1 hour, or a second link is requested and the first is posted
- **THEN** exactly the first parallel post changes the password, and every other post gets the invalid-link page without change

#### Scenario: GET never consumes
- **WHEN** a mail scanner fetches a reset link with `GET` and `HEAD` ten times
- **THEN** the token stays valid and the password is unchanged

### Requirement: Account commands

`kohaku admin unlock --email <address>` SHALL end the account's lock and reset both
failure counts. `kohaku admin reset-password --email <address>` SHALL create a 1-hour reset
token for the account (any role, the admin included), invalidate its other unused reset
tokens and print only the reset link to stdout, sending no mail. Both SHALL exit 1 with a
message on stderr when no account has that address, and SHALL record their audit entry with
actor `cli`.

#### Scenario: Admin recovers from a lock or a forgotten password
- **WHEN** the operator runs `kohaku admin unlock --email admin@example.org` on a locked account, then `kohaku admin reset-password --email admin@example.org`
- **THEN** the next correct login without a device cookie succeeds, and stdout holds exactly one line, the reset link, which works like a mailed one

### Requirement: Account mail content

Every mail of this change SHALL be plain text from fixed wording plus the main-host origin,
UTC times and, for the reset mail, the link; none SHALL contain the password, a code, the
requesting network or any user-supplied text. The lockout and password-changed mails SHALL
have security priority; the reset mail SHALL be token-bearing with an expiry equal to its
token's.

#### Scenario: Mail leaks nothing
- **WHEN** a lockout, a password change and a reset request happen for an account after an attacker put `<script>` and a CR LF sequence in the login form's email and password fields
- **THEN** no queued mail contains those values, a password, a code or an IP address
