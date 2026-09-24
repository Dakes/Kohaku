# Proposal

## Why

Moderation, projects and every admin page need a signed-in admin or maintainer, and the
admin account sees everything. Signing in is the one place where a public attacker can
guess passwords or phish sessions, so it gets built, tested and reviewed on its own,
before any page depends on it (design §8, §15 item 2).

## What Changes

- `users`, `sessions`, `known_devices`, `recovery_codes` and `tokens` tables in one
  migration; the outbox gains account recipients.
- Login on the main host (`/admin/login`) with email, password and a TOTP or recovery code
  in one form: no half-authenticated state, one generic error, argon2id behind a bounded
  pool.
- Sessions in a `__Host-` cookie (12 h idle, 7 d absolute), CSRF tokens on every admin
  form, logout.
- Lockout that strangers cannot use against the owner: a per-account lock for browsers
  without a device cookie, a separate hourly cap for browsers with one, a lockout mail at
  most daily, and `kohaku admin unlock`.
- TOTP (RFC 6238) with seeds derived from the instance secret and never stored, replay
  protection, 10 single-use recovery codes, enrollment and re-enrollment from the account
  page with a QR code any authenticator app scans (FreeOTP+ included).
- Re-authentication (current password plus code) before changing the password, disabling
  or re-enrolling 2FA or regenerating recovery codes.
- Password reset by mail link (maintainers only; 1 h, single use, never skips TOTP) and
  `kohaku admin reset-password` for any account.
- `login` and `reset` rate-limit classes and a per-address reset limit.
- A minimal signed-in home at `/admin` and the account page.

## Capabilities

### New Capabilities

- `admin-auth`: accounts, login, sessions, CSRF, lockout, two-factor authentication,
  re-authentication and password reset.

### Modified Capabilities

- `host-routing`: the main router serves the sign-in and account routes; cookies exist now.
- `request-limits`: `read` covers admin GETs; new `login` and `reset` classes.
- `mail-outbox`: rows may name a user account as recipient.
- `audit-log`: the account actions of this change are audited.
- `data-storage`: retention also deletes expired sessions, tokens and devices.
- `operations`: `kohaku admin unlock` and `kohaku admin reset-password`.
- `configuration`: the settings the two new commands need.

## Non-goals

- Creating accounts: `admin create`, setup links, invites, grants, disabling users and
  "require 2FA for maintainers" arrive with `users-and-invites`. Tests create accounts
  directly.
- Access guards per project (`RequireAdmin`, `ProjectAccess`) and any page beyond the
  signed-in home and the account page.
- WebAuthn/passkeys, SMS or mail codes as a second factor, "remember me", login by mail
  link, changing one's email address, CAPTCHA.
- Audit entries for logins and failed logins (they need neither a session nor a token;
  the audit-log spec excludes them).

## Security considerations

- **Attacker: online password guesser.** Per-network `login` buckets (10 per 15 min per
  /32 or /64, 80 per /48), a per-account lock after 10 failures, a dummy hash for unknown
  and locked accounts so timing and responses match, one generic error for every failure.
- **Attacker: locks the owner out on purpose.** Browsers that logged in before carry a
  device cookie; the per-account lock never applies to them, and they have their own
  bounded budget (20 failures per hour) and a reserved argon2 permit, so a flood cannot
  starve the owner.
- **Attacker: phished or stolen password.** TOTP is required for the admin; codes are
  replay-protected; a password reset never removes TOTP and never creates a session.
- **Attacker: leaked database or backup.** Password hashes are argon2id (19 MiB, t=2);
  TOTP seeds are derived from the instance secret, not stored; session, device, reset and
  recovery values are stored as SHA-256 only.
- **Attacker: cross-site requests and session theft.** `__Host-` cookies with
  `SameSite=Strict` and `HttpOnly`, on the main host only; CSRF tokens plus the
  Fetch-Metadata/Origin check; sessions rotate on login and die on password change.
- **Attacker: mail-bombing via reset.** `reset` class (5 per hour per network), 3 per hour
  per address, the shared public mail budget; unknown, admin and placeholder addresses get
  the same response and the same work.
- **Attacker: resource exhaustion.** Two argon2 permits (one reserved), a 2 s wait, then
  503; hashing inputs capped at 1024 characters.

## Impact

Migration 2 (`users`, `sessions`, `known_devices`, `recovery_codes`, `tokens`; `outbox`
rebuilt with a `user_id` foreign key). New modules for accounts, sessions, TOTP and the
admin pages; templates for login, reset, home and account. Two CLI commands.

New dependencies (justified in `docs/dependencies.md`):

- `argon2`: argon2id password hashing (RustCrypto; no C).
- `sha1`: HMAC-SHA1 as RFC 6238 TOTP requires (authenticator apps only support SHA-1).
- `qrcodegen`: QR code for TOTP enrollment (Nayuki; MIT, no dependencies, no build script);
  Kohaku draws its output as inline SVG.
- `password-hash` comes with `argon2` for the PHC string format; no other new crates.
