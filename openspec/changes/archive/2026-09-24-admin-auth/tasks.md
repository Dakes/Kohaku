# Tasks

## 1. Schema and accounts

- [x] 1.1 Add `migrations/0002_admin_auth.sql` (D2) with the outbox rebuild; verify the §12
  migration test with sample rows in every table, the one-admin index refusing a second
  admin, and mail-outbox "Account rows follow the account".
- [x] 1.2 Add `argon2` and `sha1` (D1; `qrcodegen` joins in 6.3) with `docs/dependencies.md` and `deny.toml` entries;
  verify `cargo deny check` and that the tree gains no build script outside the allow-list.
- [x] 1.3 Add account email normalization, the password rule and argon2id hashing with the
  startup dummy hash (admin-auth: Accounts); verify unit tests and "Attacker holds a
  database copy" for the hash format.
- [x] 1.4 Add the `test_support` account helper and injected clock for this change (D13).

## 2. Keys and TOTP

- [x] 2.1 Add `Purpose::Totp` and `Purpose::MailAddress` (D8, D9; design §5 already lists
  both); verify label uniqueness and "Seed is not in the data directory".
- [x] 2.2 Add RFC 6238 TOTP with the ±1-step window and the replay write (D8); verify the
  RFC 6238 SHA-1 test vectors, "Replayed code" and the parallel half of "Codes race in
  parallel".
- [x] 2.3 Add recovery codes: generation, display format, input normalization, consuming
  delete (D8); verify the recovery-code half of "Codes race in parallel".

## 3. Limits

- [x] 3.1 Extend `read` to admin GETs and add the `login` and `reset` classes
  (request-limits: Read rate-limit class, Login and reset rate-limit classes); verify both
  new scenarios and matrix rows per route.
- [x] 3.2 Add the per-address bucket (request-limits: Per-address reset limit); verify its
  scenario and that the address appears in no log or row.
- [x] 3.3 Add the two hashing permits with the 2 s wait (D6; admin-auth: Password hashing
  is bounded); verify "Login flood cannot starve the owner".

## 4. Sessions, CSRF and routing

- [x] 4.1 Add the `Public`/`Session` access declaration to the route table and matrix (D3);
  verify a route without it fails to compile and "Attacker enumerates admin paths".
- [x] 4.2 Add the session guard with idle, absolute expiry and 5-minute use recording (D4;
  admin-auth: Sessions); verify "Idle and absolute expiry" and that a disabled or
  passwordless account's session is rejected.
- [x] 4.3 Add CSRF tokens and the check (admin-auth: CSRF tokens); verify "Attacker forges
  a same-site form post" and a matrix row for every `Session` POST.
- [x] 4.4 Add `admin_base.html`, `/admin` home and logout (D12); verify logout deletes the
  session row and expires the cookie, and host-routing "Attacker probes for the admin
  area".

## 5. Login and lockout

- [x] 5.1 Add the login page and pipeline (D5; admin-auth: Login); verify "Attacker probes
  accounts", "Attacker has the password but not the code", "No redirect target from the
  request", the 2FA-required page for an unenrolled admin, and "Attacker fixes or replays a
  session".
- [x] 5.2 Add device cookies and the lockout writes (D7; admin-auth: Lockout); verify its
  four scenarios, device-cookie eviction at 10, and host-routing "Cookies only from login
  and logout, even for a forged one".
- [x] 5.3 Add the lockout and password-changed mail kinds (D10; admin-auth: Account mail
  content); verify the mail bodies and "Mail leaks nothing".

## 6. Account page

- [x] 6.1 Add the account page and re-authentication (admin-auth: Re-authentication for
  sensitive account changes); verify "Stolen session cannot take over the account".
- [x] 6.2 Add password change (sessions ended, devices deleted, mail, audit
  `user.password_change`); verify each effect.
- [x] 6.3 Add TOTP enrollment with the key URI and inline-SVG QR code (adds `qrcodegen`),
  re-enrollment, disable and recovery-code regeneration with their audit actions (D1, D8);
  verify "Scanning the enrollment QR code" (FreeOTP+ by hand, recorded in the PR),
  enrollment ends other sessions, disable is refused for the admin, and codes are shown
  once.
- [x] 6.4 Add "end other sessions" with `user.sessions_end`; verify the current session
  survives and the others do not.

## 7. Password reset

- [x] 7.1 Add the reset request (D9; admin-auth: Password reset); verify "Attacker probes
  accounts through reset", request-limits "Attacker mail-bombs one mailbox from many
  networks", and that a request with a forged `Host` and `X-Forwarded-Host` still mails a
  `KOHAKU_BASE_URL` link (design §12).
- [x] 7.2 Add the reset link pages and consumption with `user.password_reset`; verify
  "Attacker with the victim's mailbox", "Reset link replayed or raced" and "GET never
  consumes", and that TOTP, recovery codes and lock state are unchanged.
- [x] 7.3 Add the reset mail kind with its give-up action (D10); verify a given-up reset
  row makes its link invalid.

## 8. CLI, retention and documentation

- [x] 8.1 Add `admin unlock` and `admin reset-password` (D11; admin-auth: Account commands;
  operations: Command-line commands; configuration: Startup validation and error
  reporting); verify "Admin recovers from a lock or a forgotten password", "Malformed admin
  invocations", the reset-password configuration case and audit entries with actor `cli`.
- [x] 8.2 Add the retention steps (data-storage: Retention job); verify "Expired sign-in
  state is purged".
- [x] 8.3 Extend `tests/logging.rs` with login, reset and account requests carrying marker
  passwords, codes, tokens and addresses; verify none reaches the log.
- [x] 8.4 README: signing in, 2FA, lockout and recovery (`admin unlock`,
  `admin reset-password`); verify against the shipped behaviour.
- [ ] 8.5 Final check: foundation's before-finishing commands, the smoke test, and a
  real-browser login with TOTP from an authenticator app, recorded in the PR.
