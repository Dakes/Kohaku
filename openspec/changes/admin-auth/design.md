# Design

## Context

Builds on `foundation` (route table and matrix, header/cache/origin layers, `db.read` /
`db.write`, key registry, outbox, audit, limiter, CLI grammar). Design §8 is the source;
this document adds mechanisms and resolves what §8 leaves open. Accounts are created by
`users-and-invites`; until then tests insert them directly.

## Goals / Non-Goals

**Goals:** every credential check runs the same work whatever the outcome; every
counter and single-use value is decided by one conditional write; no page beyond login,
reset, home and account.

**Non-Goals:** see proposal. Also no "remember this browser" choice (the device cookie is
automatic), no session list with details (no IPs or user agents are stored).

## Decisions

**D1. Dependencies.** `argon2` (RustCrypto, `default-features = false`, features `alloc`
and `password-hash` for PHC strings; salts from `getrandom`, no `rand`), `sha1`
(`default-features = false`) for HMAC-SHA1 over the existing `hmac`, `qrcodegen` for the
enrollment QR code (owner decision; D8). axum gains its `form`
feature (first route parsing a form body), which brings `serde_urlencoded`. Each justified
in `docs/dependencies.md`; licenses and build scripts checked against `deny.toml`.

**D2. Schema (`migrations/0002_admin_auth.sql`).**

- `users`: `id INTEGER PRIMARY KEY AUTOINCREMENT`, `email TEXT UNIQUE`, `password_hash`
  (NULL until set), `role CHECK (role IN ('admin','maintainer'))`, `disabled`,
  `totp_nonce BLOB` (16 bytes or NULL), `totp_last_step`, `failed_logins`, `locked_until`,
  `device_failures`, `device_window_start`, `lockout_mailed_at`, `created_at`;
  `CREATE UNIQUE INDEX users_one_admin ON users(role) WHERE role = 'admin'`.
- `sessions`: `token_hash BLOB UNIQUE` (SHA-256), `user_id` → users `ON DELETE CASCADE`,
  `csrf_token BLOB`, `created_at`, `last_seen`, `pending_totp_nonce BLOB`.
- `known_devices`: `user_id` (cascade), `token_hash UNIQUE`, `created_at`; index on
  `user_id`.
- `recovery_codes`: `(user_id, code_hash)` primary key, cascade.
- `tokens`: `id INTEGER PRIMARY KEY AUTOINCREMENT` (invites appear in audit entries),
  `purpose CHECK (purpose IN ('reset','setup','invite'))`, `token_hash UNIQUE`, `user_id`
  (cascade, NULL for invites), `email`, `totp_nonce BLOB` (setup and invite pages show a
  QR code without writing on GET), `expires_at`, `used_at`. Only `reset` rows exist until
  `users-and-invites`; the table is final so that change adds no rebuild.
- `outbox` rebuilt with `user_id REFERENCES users ON DELETE CASCADE` (foundation D31.4),
  under the migration runner's foreign-keys-off rebuild; its §12 sample-row test gains the
  new tables.

**D3. Routes.** All on the main router, all `no-store`, 64 KiB, 15 s, not exempt from the
origin check. Access is a new route-table declaration, `Public` or `Session`, which the
matrix enumerates.

| Route | Access | Rate class |
|---|---|---|
| `GET /admin/login`, `POST /admin/login` | Public | read / login |
| `POST /admin/logout` | Session | none |
| `GET /admin` | Session | read |
| `GET /admin/account` | Session | read |
| `POST /admin/account/password` | Session, re-auth | login |
| `POST /admin/account/totp` (start, re-auth), `…/totp/confirm` | Session | login |
| `POST /admin/account/totp/disable`, `…/recovery-codes` | Session, re-auth | login |
| `POST /admin/account/sessions/end-others` | Session | none |
| `GET /admin/reset`, `POST /admin/reset` | Public | read / reset |
| `GET /admin/reset/{token}`, `POST /admin/reset/{token}` | Public | read / login |

A `Session` route without a valid session answers per the admin-auth spec before its
handler; the main router's `/admin/…` fallback does the same, so unknown paths match known
ones. POSTs without session to non-existent `/admin` paths are 403 by the same rule.
Logout and ending other sessions take no rate token: they need a session and CSRF.

**D4. Session guard.** Reads the cookie (exactly 43 base64url characters, else no session,
no query), hashes it, looks it up on a reader, checks idle, absolute expiry and account
state, and writes `last_seen` on the writer when it is ≥ 5 min old. The guard hands the
handler `SessionUser { user_id, role, email, csrf, session_id }`; the CSRF check runs in
the same extractor for every non-GET/HEAD `Session` route, with one constant-time byte
comparison written once (`keys::ct_eq`, no new crate).

**D5. Login pipeline.** In order: body cap → `login` token → parse form (email ≤ 254,
password ≤ 1024 scalar values, code ≤ 32 characters; longer counts as a failure, not 422) →
normalize email → reader: account, device-cookie match → argon2 permit (D6) → writer:
lock state re-read → argon2 verify in `spawn_blocking` (the dummy hash when the account is
unknown, disabled, passwordless or locked for this request) → code check → one writer
transaction: success (reset counts, new session, device row and eviction, `totp_last_step`
or recovery-code delete) or failure (the counting UPDATE of D7). The dummy hash is an
argon2id PHC string of a random password made at startup with the production parameters.

**D6. Hashing permits.** Two semaphores of one permit: `general` and `reserved`. A request
with a valid device cookie for the submitted account tries `reserved`, then `general`;
others only `general`; `tokio::time::timeout(2 s)` around the acquisition, 503 on expiry.
Re-authentication uses the same path (its account is the session's).

**D7. Lockout writes.** Failure without device cookie:
`UPDATE users SET failed_logins = CASE WHEN locked_until <= :now THEN 1 ELSE failed_logins + 1 END, …`
with the 10th setting `locked_until = :now + 900`, `failed_logins = 0` and, if
`lockout_mailed_at` is NULL or ≤ `:now - 86400`, `lockout_mailed_at = :now` — `RETURNING`
whether the lock started and whether to mail, then `enqueue` in the same transaction.
While `locked_until > :now` the UPDATE's `WHERE` excludes the row (no count, no extension).
Failure with device cookie: the window variant on `device_failures`/`device_window_start`.
Success: counts to 0, `locked_until` untouched.

**D8. TOTP.** `keys::Purpose::Totp` (`kohaku/totp`) with fields account id (8-byte BE) and
nonce; first 20 bytes; base32 (RFC 4648, no padding, 32 characters) for display;
`otpauth://totp/Kohaku%20(<main%20host>)%3A<email>?secret=…&issuer=…&algorithm=SHA1&digits=6&period=30`,
percent-encoding everything but RFC 3986 unreserved characters. The QR code (`qrcodegen`,
ECC M, boosted) becomes one inline `<svg>` with a white `<rect>` and one `<path>` of unit
squares, colours as presentation attributes (not `style`), so the CSP stays as it is;
a test checks the SVG's squares against `qrcodegen`'s module matrix for the URI, and
FreeOTP+ scans it once by hand.
Replay: `UPDATE users SET totp_last_step = :step WHERE id = :id AND (totp_last_step IS NULL OR totp_last_step < :step)`.
Enrollment start stores `pending_totp_nonce` on the session (one pending enrollment per
session, replaced by a new start); confirm moves it to `users`, resets `totp_last_step` to
the confirmed step, replaces recovery codes, ends other sessions and deletes device rows,
in one transaction. Recovery codes display as `XXXX-XXXX-XXXX-XXXX`; input is uppercased
and stripped of `-` and spaces, then must be 16 base32 characters.

**D9. Reset.** Token: 32 getrandom bytes, base64url without padding, SHA-256 stored. The
path parameter is checked for shape before any query. Request order: `reset` token →
address shape (422 on violation: it reveals nothing about accounts) → per-address bucket
(`keys::Purpose::MailAddress` under a `PerBootKey`) → public mail budget → one write
transaction: token row plus mail row, or placeholder row. Consuming:
`UPDATE tokens SET used_at = :now WHERE token_hash = :h AND purpose = 'reset' AND used_at IS NULL AND expires_at > :now RETURNING user_id`,
then the password write, session and device deletes, other tokens marked used, mail, audit —
one transaction. The reset mail kind's give-up action marks its token used.

**D10. Mail kinds.** `account.reset_link` (normal priority, token-bearing, 1 h),
`account.lockout` and `account.password_changed` (security, 24 h). Bodies are constants
with the main-host origin, a UTC time (`YYYY-MM-DD HH:MM UTC`) and the link.

**D11. CLI.** The grammar gains `admin unlock|reset-password --email <address>`. Both run
without tokio like `backup`, open the database (no instance lock; SQLite's busy timeout
orders them with a running `serve`), check the keycheck, write in one transaction with the
audit entry. `reset-password` builds the link with the URL builder.

**D12. Templates.** `admin_base.html` (header with the email, account link, logout form),
`login`, `login_2fa_required`, `reset_request`, `reset_sent`, `reset_form`,
`reset_invalid`, `reset_done`, `home`, `account`, `totp_enroll`, `recovery_codes`. No
JavaScript; every form carries `csrf` when a session exists.

**D13. Tests.** Accounts via a `test_support` helper writing rows directly (production
argon2 parameters: ~40 ms per hash, acceptable). Time is injected into the session, lock,
TOTP and token checks. Concurrency scenarios use real parallel requests against one
writer.

**D14. Deviations from the design.**

1. **Failure status 200** (§8 is silent): a re-rendered form with a fixed message.
2. **Password change ends the current session too** (§8 "revokes all"): the user signs in
   again with the new password, which also proves it.
3. **Sessions section = "end other sessions"** (§8 Pages lists sessions): nothing per
   session is stored that would be worth listing.
4. **Per-address limit in memory under a per-boot key** (§8 names only the limit): new key
   `kohaku/mail-address` added to §5's inventory; OTP can reuse it.
5. **Recovery codes accepted in the code field** (§8 implies it): no separate form.

## Risks / Trade-offs

- [argon2 at 19 MiB × 2 adds ~38 MiB peak] → Already in §3's memory budget.
- [A user who forgets the code field fails and counts a failure] → The field is labelled;
  10 failures only lock browsers without a device cookie.
- [Reset link in the URL path reaches proxy access logs] → Documented (§10); 1 h, single use.
- [Lockout mail can be triggered by strangers once a day] → Bounded to one per 24 h.
- [`admin reset-password` output is a live credential] → Printed only to the operator's
  terminal, 1 h, single use, audited.

## Migration Plan

Migration 2 runs at the first start of this version after the pre-migration copy; existing
outbox rows keep their address recipients. Rollback before a release: revert; after: §10.
