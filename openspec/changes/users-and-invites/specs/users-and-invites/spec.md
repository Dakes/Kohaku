# Spec Delta

## Purpose

How accounts come to exist and who may work on which project: the first admin by a setup
link from the command line, maintainers by mailed invitations with per-project grants,
disabling accounts, resetting a lost second factor, requiring 2FA for maintainers, and
recovering from a lost instance secret (design §8 Token flows, Access control, §3 CLI).

## ADDED Requirements

### Requirement: Setup links

A setup link SHALL be `/admin/setup/{token}` on the main host with a 256-bit random token
valid 1 hour, created only by `kohaku admin create`, `kohaku admin reset-2fa`,
`kohaku admin rekey` and the admin's "Reset 2FA" action, shown or printed and never mailed.
Creating one SHALL invalidate the account's other unused setup links. `GET` SHALL only render
the setup form for a valid token (the same "link invalid or expired" page otherwise); the form
sets a new password and, when the account must use two-factor authentication, enrolls TOTP
with a QR code as the account page does, the code typed back in the same `POST`. `POST` SHALL
take a `login` rate-limit token, validate everything, then consume the token in one
conditional write and, in the same transaction, set the password, replace the enrollment if
one was made, end the account's sessions and delete its device cookies. It SHALL create no
session: the page links to the login and shows new recovery codes once when TOTP was enrolled.

#### Scenario: Link holder races or replays
- **WHEN** a setup link is posted twice in parallel, again after success, and another after 1 hour
- **THEN** exactly one post sets the password; the others get the invalid-link page and change nothing

#### Scenario: Admin must enroll during setup
- **WHEN** the admin's setup form is posted with a valid password but no code or a wrong code
- **THEN** the form is shown again with a message, the token stays unused and no password is set

### Requirement: First admin from the command line

`kohaku admin create --email <address>` SHALL create the one `admin` account without a
password and print only its setup link to stdout. It SHALL exit 1 and create nothing when an
admin already exists or the address belongs to an account. It SHALL take no password from
arguments or the environment.

#### Scenario: Bootstrapping twice
- **WHEN** `kohaku admin create --email owner@example.org` runs on a new instance and again with another address
- **THEN** the first prints one setup link and the account cannot log in until the link is used; the second exits 1 and prints no link

### Requirement: Invitations

The admin SHALL invite a maintainer by email address with a set of project grants on
`/admin/users`. An invitation SHALL be valid 48 hours and SHALL be refused (422) for an
address that already has an account; inviting an address with a pending invitation SHALL
replace it. Kohaku SHALL mail the link `/admin/invite/{token}` as token-bearing mail with the
main-host origin and fixed wording, no project names and no other user-supplied text. The
admin SHALL see pending invitations with their expiry and SHALL be able to revoke one, which
makes its link invalid at once. `GET` on the link SHALL only render the acceptance form;
`POST` SHALL take a `login` rate-limit token and, in one transaction, consume the invitation,
create the maintainer account with the invited address and the given password (and TOTP
enrollment when maintainers must use it), grant only those projects that still exist, and
queue a security mail to the admin saying that an invitation was accepted. It SHALL create no
session.

#### Scenario: Invitation raced against revocation
- **WHEN** the admin revokes an invitation while its link is being posted
- **THEN** either the account exists and the invitation is gone, or no account exists and the link is invalid; never both an account and a live invitation

#### Scenario: Granted project deleted before acceptance
- **WHEN** an invitation grants projects A and B, B is deleted, and the invitation is accepted
- **THEN** the new maintainer is granted A only

#### Scenario: Attacker holds an old or revoked link
- **WHEN** an attacker posts an invitation link that was revoked, replaced, already used or is 48 hours old
- **THEN** each gets the invalid-link page and no account is created

### Requirement: Grants and project access

A maintainer SHALL work only on projects granted to them; the admin works on every project.
Every admin route acting on a project SHALL be under `/admin/p/{slug}/`, SHALL bind the
project from the slug and a grant check in one lookup, and SHALL answer 404, identical to a
nonexistent project, when the project does not exist or the user has no grant. Routes the
projects capability reserves for the admin SHALL stay admin-only. `/admin` SHALL list the
projects the user may work on. The admin SHALL change a maintainer's grants on the user's page.

#### Scenario: Maintainer probes an ungranted project
- **WHEN** a maintainer granted only project A requests `/admin/p/b/…` for existing project B and nonexistent project C
- **THEN** both get the same 404 and nothing reveals that B exists

#### Scenario: Grant revoked mid-session
- **WHEN** the admin removes project A from a maintainer's grants while the maintainer is signed in
- **THEN** the maintainer's next request under `/admin/p/a/` gets 404

### Requirement: Disabling accounts

The admin SHALL disable and re-enable maintainer accounts on `/admin/users`; the admin
account cannot be disabled. Disabling SHALL, in one transaction, end the account's sessions,
make its unused setup and reset links invalid and delete its unsent mail. A disabled account
cannot log in, use a link or request a reset (admin-auth: its requests look like an unknown
address's).

#### Scenario: Disabled maintainer with a live session and a reset link
- **WHEN** the admin disables a maintainer who is signed in and holds an unused reset link
- **THEN** the maintainer's next request gets `303` to the login, the link shows the invalid-link page, and logging in fails generically

### Requirement: Resetting a lost second factor

`kohaku admin reset-2fa --email <address>` (any account) and the admin's "Reset 2FA" action
on a maintainer's page SHALL, in one transaction, remove the account's TOTP enrollment and
recovery codes, end its sessions, delete its device cookies and create a setup link, which the
command prints to stdout and the page shows once.

#### Scenario: Maintainer lost their phone
- **WHEN** the admin resets 2FA for a signed-in maintainer and the maintainer completes the new setup link
- **THEN** the old session and codes stop working at once, and after setup the maintainer logs in with the new password and, if enrolled again, the new codes

### Requirement: Requiring 2FA for maintainers

The admin SHALL switch "require 2FA for maintainers" on `/admin/users`, off for a new
instance. While on, a maintainer without TOTP SHALL be treated like an unenrolled admin at
login (no session, told to ask for a setup link), SHALL NOT be able to disable TOTP, and
setup and invitation pages SHALL require enrollment. Turning it on SHALL end every session of
maintainers without TOTP in the same transaction.

#### Scenario: Requirement switched on
- **WHEN** the admin turns the requirement on while an unenrolled maintainer is signed in
- **THEN** that session ends, the maintainer's next correct password gets the "two-factor login is required" page, and an enrolled maintainer stays signed in

### Requirement: Recovering from a lost instance secret

`kohaku admin rekey`, run with the server stopped, SHALL hold the instance lock, skip the
keycheck and, in one transaction with a new `KOHAKU_SECRET`: remove every account's TOTP
enrollment and recovery codes, end every session, clear every other stored value derived from
the old secret (none in this change; later capabilities name theirs), store the new keycheck
and create a setup link for every account that had TOTP enrolled. It SHALL print one line per
such account, the account's address, a tab and its link, to stdout.

#### Scenario: Secret lost, backup restored
- **WHEN** an operator restores a backup made under a secret they no longer have, sets a new `KOHAKU_SECRET` and runs `kohaku admin rekey` while `kohaku serve` is stopped
- **THEN** it prints a setup line for each previously enrolled account, `kohaku serve` then starts under the new secret, and no old TOTP code or session works

#### Scenario: Rekey while the server runs
- **WHEN** `kohaku admin rekey` runs while `kohaku serve` holds the instance lock
- **THEN** it exits 1 saying another Kohaku process uses `/data` and changes nothing

### Requirement: Users page

`/admin/users` SHALL be admin-only (404 for maintainers) and list every account with its
address, role, whether TOTP is enrolled, whether it is disabled and its granted projects, plus
pending invitations, the invitation form and the 2FA requirement switch. No page SHALL show a
password hash, token, nonce or recovery code other than a just-created setup link or new
recovery codes shown once to their creator.

#### Scenario: Maintainer probes user administration
- **WHEN** a signed-in maintainer requests `/admin/users` and posts an invitation, a grant change and a disable with valid CSRF tokens
- **THEN** each gets 404 and nothing changes
