# Spec Delta

## MODIFIED Requirements

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

An account required to use two-factor authentication (the `admin` role, and maintainers
while "require 2FA for maintainers" is on, users-and-invites)
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
