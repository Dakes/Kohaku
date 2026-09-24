# Kohaku — Design

Date: 2026-09-23 · Status: validated; security-reviewed (2 rounds, 89 findings, see
docs/plans/2026-09-23-security-review.md)

## 1. What it is

A tiny, self-hosted bug report inbox. The admin creates projects; each project gets a
public report form, a public list of reports (open / in progress / closed) with a
permanent **Fixed** section, and a JSON API. Projects can also accept feature requests
(off by default). Reporters never register. New reports are private until a maintainer
approves them (moderated public). Aimed at small developers
(apps, SaaS, game mods). Open source, AGPL-3.0-or-later.

**Principles:** security first (public, unauthenticated input), minimal resources,
minimal dependencies, boring and auditable code.

**Non-goals:** boards, sprints, assignees, labels, public comment threads, reporter
accounts, HTML email, SPA frontend.

**Later (not v1):** switching a project to `require_email` automatically at the pending
backstop; a per-report +1 rate cap.

## 2. Decisions log

| Topic | Decision | Why |
|---|---|---|
| Backend | Rust stable, axum + tokio, single static musl binary | Fixed by owner |
| Frontend | Server-rendered askama templates, one CSS, small vanilla JS + PoW worker; no React | No npm supply chain, no JSON admin API, strict CSP. Reading and admin work without JS; submit, OTP and +1 need JS (PoW) |
| Visibility | Moderated public: `pending` is private until approved. Public = allowlist of statuses, defined once as SQLite views | Spam never goes public; a forgotten filter fails closed |
| Takedown | Private `hidden` status reachable from every state; audited maintainer edits; admin hard delete | Approved reports can be unpublished, redacted or removed |
| Anti-spam | PoW + honeypot + rate limits always; per-source pending cap; email OTP per project (optional). PoW is a cost, quotas and rate limits are the limits | Anonymous by default, stronger gate where wanted |
| Admins | One admin bootstrapped by a CLI-printed single-use setup link + maintainers invited by email with per-project grants | No password on the command line; no password-only enrollment window |
| Lockout | OWASP device cookie: after 10 failures only browsers without a device cookie from a prior login are locked | An attacker cannot lock the owner out |
| Keys | Ephemeral per-boot keys by default; one external secret (`KOHAKU_SECRET`) derives TOTP seeds and source keys | A leaked DB or backup alone yields no TOTP codes and no reversible IP hashes |
| Report content | Markdown (no raw HTML, no images), screenshots per project (off by default) | Owner request |
| Feature requests | A report has a kind, `bug` or `feature`; feature requests per project (off by default), same moderation and limits as bugs | Owner request (2026-09-24) |
| Screenshots | Decode in pure Rust, re-encode lossy WebP q80 via libwebp **encoder only**; libwebp decoding banned and enforced | Owner wants lossy WebP; untrusted encoded bytes never reach C (attacker-chosen pixel values still do) |
| Screenshot storage | Re-encoded WebP as SQLite BLOBs, not files | Cascades, purge, erasure and backup cover images; no path handling |
| Interaction | Maintainer public notes (generic "Maintainer" label) + "+1" counter (per project) | No comment threads |
| 2FA | TOTP in v1: required for admin, optional for maintainers (admin can require) | Admin account sees everything |
| TLS | Behind a reverse proxy; `KOHAKU_TRUSTED_PROXIES` is required, exact addresses (CIDR only as a last resort), no preset | Fewer crates; no fail-open IP trust |
| Spec process | OpenSpec (spec-driven schema), one change per capability | Owner request |
| Distribution | Docker Hub `dakes/kohaku`, built by GitHub Actions on bare semver tag (`1.0.0`); publish job gated by the `release` environment with owner approval; version tags immutable; cosign keyless by digest | Owner request; a pushed tag alone cannot publish |
| Dev env | Nix flake (devShell + package) and rustup both supported; OpenSpec from nixpkgs, no nodejs | Owner on NixOS; no unpinned npm on machines that push tags |
| Live reload | `dev` feature serves a ~15-line same-origin polling script; no tower-livereload, no listenfd | No weaker CSP, no extra crates |
| Domains | Optional custom domain per project on one instance; admin only on the main domain | Protects admin sessions from bugs in public-only pages. For report content the boundary is the sanitizer plus the CSP; neither may be loosened because of the split |
| Deployment | Docker Compose is the first-class path: shipped `compose.yaml` + Caddy, CI-tested, fetched from the release tag | Owner request; security behaviour lives in the image |
| Migrations | Forward-only, runner takes a pre-migration copy, refuses newer DBs | Rollback = restore + pin previous tag |
| Audit | `audit_log` from `foundation`; ids only; 1-year retention | Moderation must be audited from its first release |

## 3. Architecture

```
reverse proxy (TLS) ──► kohaku (axum, HTTP :8080, non-root, distroless static)
                          ├── header layer    outermost: security headers + Cache-Control, every host
                          ├── internal        /healthz, /.well-known/kohaku/tls-ask   (any Host)
                          ├── host map        Host → main router | project router | 421
                          │    ├── main       /admin/…, login/invite/reset/setup/unsubscribe,
                          │    │              /p/{slug}/…, /p/{slug}/api/v1/…, /static
                          │    └── project    /, /new, /r/{n}, /api/v1/…, OTP, +1, /static
                          ├── SQLite (WAL)    /data/kohaku.db  (screenshots as BLOBs)
                          ├── backups         /data/backups/   (pre-migrate copies, backup temp files)
                          ├── instance lock   /data/kohaku.lock
                          └── background      mail outbox, notification coalescing, retention, WAL checkpoint
```

- Our code: `#![forbid(unsafe_code)]`. Release profile keeps `panic = "unwind"`, so a
  panic in a decoder fails one request.
- `kohaku serve` holds an exclusive lock on `/data/kohaku.lock`
  (`std::fs::File::try_lock`, Rust ≥ 1.89); a second instance or a restore refuses.
- SIGTERM: axum `with_graceful_shutdown` (≤ 8 s), then close DB connections.
- Kohaku creates `/data/backups` (0700) when it first needs it.

### Config

- Env vars (primary, Compose-friendly), optional TOML file. Never configurable via web
  UI.
- Secrets are environment variables like every other setting (`KOHAKU_SECRET`,
  `KOHAKU_SMTP_PASSWORD`), kept in Compose's `.env` (mode 0600; owner decision: one
  place for all settings). Never taken from arguments, never logged or echoed; errors
  name the variable, never its value. Values are used exactly as given, never trimmed.
- Required, no default: `KOHAKU_BASE_URL`, `KOHAKU_TRUSTED_PROXIES`,
  `KOHAKU_SECRET` (see §5), SMTP settings. Startup fails on unchanged example
  values.
- `KOHAKU_BASE_URL` must be exactly `https://host[:port]`: no path, query or fragment.
  `http://localhost` only with the `dev` feature.
- Optional settings, each with a documented default used when unset (owner decision
  2026-09-24: only values that differ per deployment are settings):
  `KOHAKU_PUBLIC_MAIL_PER_HOUR` (60; depends on the SMTP provider),
  `KOHAKU_SCREENSHOT_QUOTA_PROJECT_MIB` (256) and `KOHAKU_SCREENSHOT_QUOTA_INSTANCE_MIB`
  (1024; depend on the disk). Everything else is a named constant in code: pending caps
  (5 per source per project, 500 per project), rate-limit budgets, PoW difficulties.

### CLI

`kohaku serve | healthcheck | backup <file>|- | restore <file>|- | restore --list`
`kohaku admin create --email X | reset-2fa --email X | reset-password --email X | unlock --email X | rekey`
`kohaku project create <slug> --name <n> [--host <h>]`

- The image has `kohaku` on PATH and no ENTRYPOINT (§13), so `docker compose exec|run
  … kohaku <cmd>` works. With `-`, `backup` writes to stdout and `restore` reads stdin;
  CLI commands log to stderr only (§10).
- `healthcheck` probes `http://127.0.0.1:8080/healthz` (distroless has no shell or curl).
- `admin create` takes no password, refuses if an admin exists, prints a single-use
  1 h setup link (§8). Passwords are never taken from argv or env.
- `reset-2fa` clears enrollment, revokes sessions, audits with actor `cli`, prints a new
  setup link. `reset-password` prints a 1 h reset link (web reset is disabled for the
  admin role). Links are built from `KOHAKU_BASE_URL`.
- `admin rekey` recovers from a lost secret, run with the service stopped (`docker
  compose run --rm kohaku kohaku admin rekey`). It is the only command that skips the
  keycheck: with the new `KOHAKU_SECRET` it clears every user's `totp_nonce`,
  `totp_last_step` and recovery codes, revokes all sessions, NULLs `source_key`, writes
  the new keycheck, audits with actor `cli` and prints setup links for users who were
  enrolled.
- `project create` exists for the smoke test and scripted setups; container access is
  already full access, so it grants nothing new. A `--host` reaches the running
  server's host map within 2 s (§6).
- Every CLI command that changes state writes an audit entry with actor `cli`.

### DB access

- rusqlite (bundled) on a small `spawn_blocking` pool: one writer connection, 4 readers
  and one host-map watcher (§6). `STRICT` tables.
- Pragmas on every connection: `journal_mode=WAL`, `synchronous=NORMAL`,
  `foreign_keys=ON`, `busy_timeout=5000`, `journal_size_limit=67108864`. Writer also
  `secure_delete=ON`.
- **Check-and-consume rule:** every check-and-consume and check-and-count is one
  conditional statement or transaction on the writer, decided by rows affected or
  `RETURNING`: OTP attempts, login failures, TOTP step, recovery codes, tokens, pending
  caps (enforced by the report insert itself, §6). Security decisions never come from a
  reader snapshot; reader checks are only early rejects. The one allowed overshoot is
  the screenshot quota (§6).
- `sessions.last_seen` is written at most once per 5 min per session.
- Public writes (submit, OTP, +1) take a permit from a semaphore of 8
  (`try_acquire`, else 503). Admin writes bypass it.
- Background worker runs `PRAGMA wal_checkpoint(TRUNCATE)` hourly, after the retention
  job and after any erasure.

### Migration rules

Enforced by the runner, not by individual migrations (`PRAGMA user_version`):

1. **Downgrade guard:** exit with a clear error when `user_version` is higher than the
   newest migration the binary knows.
2. **Pre-migration copy:** before applying anything, `VACUUM INTO
   /data/backups/pre-migrate-v{old}-{unixtime}.db`, mode 0600. Keep the 2 newest.
3. Per migration: `PRAGMA foreign_keys=OFF` outside any transaction (bundled SQLite
   defaults it ON; the pragma is a no-op after BEGIN) → BEGIN → migration + `user_version`
   bump → `PRAGMA foreign_key_check` (any row: roll back, exit) → COMMIT →
   `foreign_keys=ON`.
4. One migration per change; never edit a migration that shipped in a release.

### Client IP

One module, about 40 lines:

- Client-IP resolution, its warnings and the rate limiter run only for requests routed
  through the host map; `/healthz` and `/.well-known/kohaku/tls-ask` are matched before
  it and never resolve a client address.
- Canonicalize every address with `IpAddr::to_canonical()` before CIDR matching, masking
  or hashing (IPv4-mapped peers from a `[::]` listener).
- `KOHAKU_TRUSTED_PROXIES`: exact proxy addresses (/32, /128); CIDRs allowed but
  documented as last resort; literal `none` = direct exposure.
- Read only `X-Forwarded-For`, never `Forwarded`, `X-Real-IP`, `X-Forwarded-Host` or
  `X-Forwarded-Proto`. Join all XFF headers, walk right to left skipping trusted
  entries; the first untrusted entry is the client. Any malformed entry: ignore the
  header, use the peer. XFF from an untrusted peer is ignored.
- One-time warnings, no IPs: "XFF from untrusted peer", "trusted peer sent no XFF",
  "client address is loopback/RFC 1918/ULA/link-local" (in the shipped setup this means
  gateway collapse or config drift).

### Rate limiter

- In-memory token buckets per action class. Keys: IPv4 /32; IPv6 /64 bucket plus a /48
  aggregate bucket in the same map with 8× the /64 budget. A bucket holds the full
  budget and refills evenly over the window.

  | Class | Routes | Budget per IPv4 /32 or IPv6 /64 |
  |---|---|---|
  | `read` | public GETs (not `/static`), unsubscribe GET and POST | 300 per minute |
  | `submit` | report submit (HTML and API) | 10 per hour |
  | `otp` | OTP send and verify | 20 per hour |
  | `plus_one` | +1 | 30 per hour |
  | `login` | login, and invite/reset/setup token POSTs | 10 per 15 min |
  | `reset` | reset request | 5 per hour |

- Public mail (OTP send, reset request) additionally takes one token from a mail bucket
  per IPv6 /48 or IPv4 /32 (10 per hour) and from the global
  `KOHAKU_PUBLIC_MAIL_PER_HOUR` bucket (60); either empty → 429.
- Every 60 s sweep entries that refilled to full. Map capped at 100 000 entries; past
  the cap new keys share one overflow bucket per class.
- Restarts reset buckets and PoW state by design; lockout counters live in SQLite.

### Concurrency and memory bounds

| Bound | Size | When full |
|---|---|---|
| Screenshot pipeline (decode → thumbnail → encode) | 2 permits, fixed | 503 + `Retry-After` |
| Multipart submissions (upload buffering ≤ ~100 MiB) | 4 permits | 503 |
| Public DB writes | 8 permits | 503 |
| argon2 (19 MiB each) | 2 permits, 1 reserved for known devices (§8); acquire with a 2 s timeout | 503 |
| DB connections | 1 writer, 4 readers, 1 watcher | wait, within the request deadline |
| tokio blocking pool | `max_blocking_threads` 32 | queued |
| Used-set (PoW nonces, verified-email ids) | 200 000 entries | 503 |
| Rate limiter map | 100 000 keys | shared overflow bucket |
| Request headers | 10 s read timeout (hyper `header_read_timeout`) | connection closed |
| Request deadline, body included (tower-http `TimeoutLayer`) | 15 s on 64 KiB routes, 120 s on the multipart route | 408, permits released |

Documented peak RSS: 2 image jobs × 160 MiB + 2 argon2 × 19 MiB + ≤ 100 MiB upload
buffers + ~64 MiB base ≈ 522 MiB → Compose `mem_limit: 640m`. The per-job figure is
measured per format (§12).

### Crates (each justified in `docs/dependencies.md`)

axum, tokio, tower-http (limits, timeouts, headers, trace) · rusqlite (bundled) ·
askama · argon2, hmac, sha2, sha1 (TOTP), qrcodegen (enrollment QR), getrandom · lettre (rustls + ring +
webpki-roots, no native-tls/aws-lc) · pulldown-cmark, ammonia · image (png/jpeg/webp
decoders only), webp (libwebp encoder only) · serde, serde_json, toml · tracing,
tracing-subscriber. Dev-only: proptest if needed.

- `default-features = false` everywhere unless a feature is needed.
- `idna_adapter = "~1.1"` pinned to select the unicode-rs backend (removes ~20 ICU4X
  crates; IDN still works). Comment in `Cargo.toml` + `docs/dependencies.md`.
- `docs/dependencies.md` records the bundled SQLite and vendored libwebp versions.
- CA roots: webpki-roots compiled in via lettre; the image's CA bundle is unused.

## 4. Data model

- `users` — email (trimmed, ASCII-lowercased, unique), argon2id hash (NULL until setup
  completes; such an account cannot log in), role `admin|maintainer`, disabled,
  `totp_nonce` (16 random bytes, NULL if not enrolled), `totp_last_step`,
  `failed_logins`, `locked_until`, `device_failures`, `device_window_start`,
  `lockout_mailed_at` (§8)
- `recovery_codes` — user_id, SHA-256(code)
- `known_devices` — user_id, SHA-256(device token), created_at; ≤ 10 per user, oldest
  evicted
- `project_members` — maintainer ↔ project grants, `notify` (per-project mail opt-out)
- `tokens` — purpose `invite|reset|setup`, SHA-256(token), user_id or email, grants
  (invite only), expires_at, used_at. Every lookup includes purpose
- `sessions` — SHA-256(token), user, CSRF token (random), created, last_seen, expiry
- `projects` — slug, name, `public_host` (optional, unique), `screenshots_enabled`
  (default off), `features_enabled` (default off), `require_email`, `plus_one_enabled`, `privacy_notice` and
  `security_contact` (optional strings, §6), `next_number` (report numbers are never
  reused, even after a hard delete)
- `reports` — per-project number, kind `bug|feature` (`feature` only while the project has
  `features_enabled`), title, markdown source, status, close_reason,
  plus_one_count, `source_key` (pending only), `approved_at` (first approval),
  `status_changed_at`, timestamps. No contact data
- `report_contacts` — report_id PK `REFERENCES reports ON DELETE CASCADE`, email,
  notify, unsubscribe_token_hash; deleted 30 days after the report enters fixed,
  closed, hidden or spam (§10)
- `report_notes` — public maintainer notes (markdown); author recorded only in the audit
  log
- `screenshots` — random 128-bit id, report (cascade), dims, WebP bytes as BLOB (last
  column)
- `plus_ones` — (report_id, voter_hash) unique, epoch
- `email_codes` — verification_id, HMAC(email), HMAC(code), attempts, expiry
- `outbox` — kind, recipient `user_id` or address (exactly one set), `report_id` (NULL
  unless about one report, `REFERENCES reports ON DELETE CASCADE`), `project_id` (NULL
  for account and security mail, `REFERENCES projects ON DELETE CASCADE`), body,
  priority (security first), attempts/backoff, `expires_at`, token-bearing flag
- `audit_log` — actor_id (no FK, history survives user deletion), actor_label, action,
  target_type, target_id, time. Ids only: never emails, titles, bodies or IPs
- `meta` — HMAC(secret, "kohaku/keycheck"), `require_2fa_maintainers`

### Lifecycle

```
submit → pending (private) ──reject──► spam (private; screenshots deleted now, text purged after 30 days)
            │ approve
            ▼
          open ⇄ in_progress ──► fixed (permanent "Fixed" section)
            ▲         └────────► closed (won't fix / duplicate; public with reason)
            └── reopen ───────── fixed | closed

any state ──hide──► hidden (private, never purged) ──unhide──► open
```

| From | To | Who |
|---|---|---|
| pending | open (approve), spam (reject) | maintainer |
| open | in_progress, fixed, closed | maintainer |
| in_progress | open, fixed, closed | maintainer |
| fixed, closed | open (reopen) | maintainer |
| any | hidden | maintainer |
| hidden | open | maintainer |
| any | deleted (hard delete) | admin |

- Every transition sets `status_changed_at` and writes an audit entry. Every
  transition out of `pending` (approve, reject, hide) clears `source_key`. Approve sets
  `approved_at`.
- Unhiding a report with `approved_at` NULL (hidden straight from pending or spam) is
  audited as an approval and sets `approved_at`. Spam → hidden is allowed; its
  screenshots stay deleted. Hidden reports are never purged and never count toward the
  pending caps.
- Hard delete removes the report row; notes, screenshot BLOBs, `plus_ones` rows,
  `report_contacts` and the report's outbox rows go with it (`ON DELETE CASCADE`).
- Maintainers may edit title, body and notes on granted projects; each edit is audited
  (ids only).

### Visibility

Public = `status IN ('open','in_progress','fixed','closed')`, defined once as SQLite
views `public_reports`, `public_notes` and `public_screenshots` (joined through
`public_reports`). Every public and API handler, the screenshot handler and the +1
write read only through these views, from one module. A non-public report returns the
same 404 as a nonexistent one.

Public templates and the API serialize only dedicated `PublicReport` / `PublicNote`
structs with no user or contact fields.

### Access control

- Two guards, declared per route in the route table: `RequireAdmin` for global admin
  routes, `ProjectAccess<Cap>` with `Cap = Moderate | ProjectAdmin`. Non-members get
  **404**.
- Every admin route acting on a report, note or screenshot is nested under
  `/admin/p/{slug}/r/{number}/…`. Handlers take project_id only from `ProjectAccess`;
  child queries bind it (`WHERE project_id=? AND number=?`, notes and screenshots joined
  through reports). Nothing is looked up by global id alone; no hidden `report_id` form
  fields.
- The dashboard and any cross-project list filter by the user's `project_members`
  rows; the admin sees everything.
- Maintainers (granted projects): moderate, bulk reject, status, notes, edit, hide,
  delete screenshots. Admin only: projects, settings, invites, grants, audit log, hard
  delete, contact erasure.

### Field limits

Violations get 422 (counted in characters unless stated).

| Field | Rule |
|---|---|
| Report title | 1–200, single line; control, bidi-override and invisible format characters rejected |
| Report body, note | markdown source ≤ 20 KiB (20480 bytes), §6 Markdown |
| `close_reason` | ≤ 200, single line |
| Project slug | `[a-z0-9][a-z0-9-]{0,39}`; none reserved (all live under `/p/`) |
| Project name | 1–80, single line |
| `security_contact` | ≤ 254, single line |
| `privacy_notice` | ≤ 1 KiB plain text |
| Email address | ≤ 254, one `@`, no whitespace or control characters |
| Password (setup, invite, reset, change) | 12–1024 (NIST 800-63B), no composition rules |

## 5. Key inventory

| Key | Lifetime | Used for |
|---|---|---|
| PoW MAC key | random per boot, memory only | PoW challenges |
| Verified-email key | random per boot, memory only | verified-email token |
| OTP key | random per boot, memory only | `email_codes` HMACs |
| Mail-address limiter key | random per boot, memory only | per-address mail limits (reset; OTP may reuse), label `kohaku/mail-address` |
| +1 epoch key | random, memory only, rotated every 24 h | voter_hash |
| Instance secret (`KOHAKU_SECRET`) | persistent, external (environment) | derives `kohaku/totp` seeds, `kohaku/source` subkey, `kohaku/keycheck` |
| — (no key) | — | CSRF (random per session), unsubscribe (random token, stored hashed), device cookies, session and token-table tokens, recovery codes (SHA-256) |

- `KOHAKU_SECRET` is required: base64 text decoding to ≥ 32 random bytes, set in
  `.env`. Never stored in the DB or `/data`.
  `serve` and every command that uses the database fail if the `meta` keycheck does
  not match, except `restore` (it replaces the database) and `admin rekey`.
- Every MAC input starts with a purpose label and uses length-prefixed fields. Every
  check uses `Mac::verify_slice`.
- TOTP seeds are never stored: seed = HMAC-SHA256(secret, "kohaku/totp" ‖ user_id ‖
  totp_nonce)[..20]. Re-enrollment draws a new nonce. A lost secret is recovered with
  `kohaku admin rekey` (§3 CLI): everyone re-enrolls 2FA. A backup only starts with the
  secret it was made with, or after a rekey.
- A per-boot key means a restart invalidates outstanding PoW challenges, OTP codes and
  verified-email tokens instead of making spent ones replayable.

## 6. Public surface

### Domains and host routing

- `KOHAKU_BASE_URL` is the **main domain**: admin area plus projects without a custom
  domain at `/p/{slug}`.
- A project may have a `public_host` (e.g. `bugs.pocetude.com`), set by the admin. On
  that host the project is served at the root (`/`, `/new`, `/r/{n}`, `/api/v1/…`).
- The host map (in memory) normalizes `Host` (port stripped, lowercased) and picks the
  main router or the project router. An admin-UI host change rebuilds it at once; the
  background worker also reads `PRAGMA data_version` on its watcher connection every
  2 s and rebuilds the map when it changed, so CLI changes (`project create --host`,
  another process) apply within 2 s.
- The project router gets the host's project_id as state and has no slug parameter:
  `/p/*`, `/admin`, login, invite, reset, setup and unsubscribe paths are 404 there.
  Unknown hosts get **421** and are never routed to the main host. Every public lookup
  on a project host binds that project_id.
- Exactly two internal endpoints are matched before the host map, for any Host:
  - `/healthz`.
  - `/.well-known/kohaku/tls-ask?domain=…` (Caddy's ask arrives with Host
    `kohaku:8080`): same normalization as the router, answers only from the host map
    (no DB), 200 for a configured host, 404 otherwise; outside every rate-limit class,
    not logged per request. It reveals only what DNS reveals.
- Canonical redirect: `/p/{slug}` → `public_host` is **308** only for GET/HEAD of HTML
  pages, with `Cache-Control: max-age=3600`, so a host change takes effect within an
  hour. Form POSTs and all of `/p/{slug}/api/v1/*` stay served on the main host: no
  body is ever re-sent and hard-coded in-app clients survive domain changes.
- Absolute URLs come from the URL builder using configured hosts only, **never from the
  request `Host`** or forwarded headers. Token-bearing and unsubscribe links always use
  the main host; report and project links use the current `public_host`.
- `public_host` validation: lowercase ASCII DNS name (IDN as punycode), ≤ 253 chars, at
  least one dot, not an IP literal, not the main host.
- Every cookie is `__Host-`-prefixed and set only by main-host handlers; project hosts
  never set cookies. PoW challenges, OTP `verification_id` and verified-email tokens
  travel in form fields or JSON.
- Same-site script is stopped by `__Host-`, the CSRF token and the exact Origin match,
  not by SameSite. Operators should put the admin host on a registrable domain that no
  project host or other service shares.

### Submission

Order (no body is read before the cheap checks):

1. Fetch-Metadata/Origin middleware (§7).
2. IP rate limit (/64 and /48).
3. Project lookup, then backstop and per-source pending cap as an early reject from a
   reader.
4. Read the body under the route's cap.
5. honeypot → PoW (nonce consumed) → verified-email token (if required) → field
   validation → markdown checks → screenshots (only if enabled, within quota) → insert
   `pending` (public-write permit) → queue maintainer notification.

- The insert enforces both pending caps: one writer transaction takes the number from
  `projects.next_number` and runs `INSERT … SELECT … WHERE (pending from this
  source_key) < per-source cap AND (pending in project) < project cap`, decided by rows
  affected (0 → roll back, same answer as step 3).
- Honeypot hits get the normal success response.
- Body caps via axum `DefaultBodyLimit` per route: 64 KiB default on every route (forms,
  JSON API, login, admin). Only two routes accept multipart: the HTML submit route of a
  project with `screenshots_enabled` (25 MiB total) and the one-click unsubscribe POST
  (64 KiB, also urlencoded; RFC 8058 senders post either, and the body is never parsed:
  the token in the path is the whole request). Multipart gets 415 everywhere else. The
  JSON API never accepts screenshots.
- Multipart route: the form puts text fields (honeypot, PoW, email token, title, body)
  before the file inputs; they are verified before the first file byte is read. A file
  part that arrives first gets 400. Every field is read with `field.chunk()` and a
  running counter: text ≤ 64 KiB, files ≤ 8 MiB, ≤ 3 file parts, ≤ 12 parts total.
  4-permit semaphore (`try_acquire`, else 503).
- Backstop: a source (HMAC of IPv6 /48 or IPv4 /32 with the `kohaku/source` subkey)
  may have ≤ 5 pending reports per project (constant; "too many
  pending reports from your network"). At 500 pending (constant)
  the form closes, and reopens as soon as moderation brings the count below; no state
  is stored. Moderators clear floods with bulk actions (§8).

### Proof of work

- Signed challenge: purpose (`submit | email_code | plus_one`), project_id, report number
  (plus_one only), 128-bit nonce, difficulty, expiry 10 min. MAC key per boot (§5).
- Verifier rejects a mismatched purpose or project and checks that purpose's
  difficulty, a compile-time constant in leading zero bits; the embedded difficulty is
  only a client hint.
- Nonces are consumed atomically at verification, before any expensive work, in a
  `Mutex<HashMap<key, expiry>>` (the used-set) pruned at each entry's expiry, capped at
  200 000 entries in total (503 when full). Error responses carry a fresh challenge
  that the form solves in the background.
- Algorithm: SHA-256(challenge‖counter) with N leading zero bits, in a web worker that
  starts on page load and uses a small synchronous SHA-256 in project JS (not WebCrypto
  per attempt). Each purpose's constant targets a p95 solve of 2–4 s on a low-end
  phone; measured once and documented next to the constants.
- Submission, OTP send and +1 require JS; a `<noscript>` notice explains. No server
  path accepts these actions without a valid PoW.
- PoW is a cost, not a limit.

### Email OTP (per project, optional)

- Sending a code costs a PoW (`email_code`). Mail body is the code only.
- Limits: 3 codes/address/hour, keyed on the lowercased address with any `+tag`
  stripped (no provider-specific dot rules). All unauthenticated mail (OTP, password
  reset) shares `KOHAKU_PUBLIC_MAIL_PER_HOUR` (default 60) plus 10/hour per IPv6 /48 or
  IPv4 /32; exhausted → 429 "try again later".
- Sending returns a random `verification_id`. Verifying takes `verification_id` + email
  + code: one conditional UPDATE on the writer consumes an attempt (`SET
  attempts=attempts+1 WHERE verification_id=? AND attempts<5 AND expires_at>?
  RETURNING` the HMACs), then HMAC(email) and HMAC(code) are both checked with
  `verify_slice`; success deletes the row. 6 digits, 10 min, 5 attempts per id (a third
  party cannot burn someone else's code). Only HMACs stored.
- Success yields a verified-email token: HMAC(key, "report-email" ‖ project_id ‖
  verification_id ‖ email ‖ expiry) plus those fields, valid 30 min. Single use: its
  `verification_id` goes into the used-set with the token's own expiry (counting toward
  the 200 000 cap), marked just before the report insert and released if the insert
  fails. The handler takes the reporter email only from the token, never from a form
  field.
- Per-project settings, both optional: `privacy_notice` shown on the form and OTP
  step; `security_contact` shown as "report security issues privately to …".

### +1

- Lighter PoW (`plus_one`, bound to the report number); one vote per voter hash per report.
- voter_hash = HMAC(epoch_key, "plus-one" ‖ report_id ‖ prefix), prefix IPv4 /32 or IPv6
  /56 after `to_canonical()`. Rows store the epoch; other epochs are deleted at rotation
  and startup, `plus_one_count` keeps the tally. Unlinkable across reports, not
  reversible from a DB copy, one per network per report per day.
- Reads through the public views: same 404 for pending, spam, hidden and nonexistent.

### Markdown

- pulldown-cmark event filter: raw HTML → text, images removed, links pass only as
  absolute URLs with scheme http/https/mailto (no `//host`, no relative, no other
  schemes). Nesting of BlockQuote/List/Item/Table > 16 → 422. Rendered HTML > 256 KiB
  → abort, 422, before ammonia. Source ≤ 20 KiB (20480 bytes).
- Exact `ammonia::Builder`: tags limited to what the enabled pulldown-cmark options
  emit; `a` gets only `href`; `url_schemes` {http, https, mailto}; `url_relative(Deny)`;
  `link_rel(Some("nofollow noopener noreferrer ugc"))` (ammonia sets rel, since it
  rewrites it otherwise).
- Rendered in `spawn_blocking` on each detail view; no cached HTML, so sanitizer fixes
  apply to old reports. The admin view of a pending report renders links as inert text
  showing the URL.

### Screenshots

Per project, off by default; HTML form only.

1. All cheap checks and the quota pass before the first file byte is read. Above quota
   (`KOHAKU_SCREENSHOT_QUOTA_PROJECT_MIB` 256, `…_INSTANCE_MIB` 1024, from
   `SUM(length(bytes))`) the form accepts text only and the dashboard shows it. The
   quota is checked only there, so concurrent uploads may overshoot it by at most
   4 submissions × 3 files.
2. Per file: ≤ 8 MiB, ≤ 3 per report, magic bytes PNG/JPEG/WebP. Header only via
   `ImageReader::with_guessed_format()?.into_decoder()` → `dimensions()`,
   `color_type()`; 422 unless each side ≤ 16384, w×h ≤ 24 MP and the decode charge
   ≤ 96 MiB. The charge is w×h×bytes_per_pixel, so 16-bit images near the pixel cap
   fail. For JPEG, our own ~20-line marker walk up to the first SOFn adds the
   full-resolution i16 coefficient planes zune-jpeg keeps for progressive images, which
   `max_alloc` does not see: SOF0/SOF1 charge w×h×bpp, SOF2 w×h×(bpp + 2×components),
   any other SOFn → 422. Progressive JPEGs are thus capped near 11 MP (YCbCr) and 9 MP
   (CMYK).
3. Explicit `image::Limits` on every decoder: max width/height 16384, `max_alloc`
   96 MiB. First frame only.
4. Only when max(w, h) > 2560: `DynamicImage::thumbnail` to ≤ 2560 px longest edge (no
   Rgba32F intermediate, unlike `resize`; it would also upscale smaller images), then
   drop the decoded image. Then `into_rgba8()`.
5. Lossy WebP q80 via `webp::Encoder::from_rgba(…).encode_simple(false, 80.0)` (returns
   `Result`). Metadata never survives re-encoding; originals never stored.
6. One global semaphore, **2 permits**, held from decode through encode;
   `try_acquire`, else 503 + `Retry-After`; work runs in `spawn_blocking`.
   Per-job peak ≤ 160 MiB: ≤ 96 MiB decode charge + ≤ 8 MiB input copy
   (`JpegDecoder::new`) + ≤ 50 MiB thumbnail, then ≤ 25 MiB RGBA8 + encoder state; see
   §3 bounds.
7. libwebp isolation: all libwebp use lives in one `encode_webp` module and only the
   encoder API is used; untrusted encoded bytes never reach C. `webp` is used with
   `default-features = false` (its default `img` feature pulls `image` with default
   features, including rayon); `libwebp-sys` is never a direct dependency. `clippy.toml`
   `disallowed-types`: `webp::Decoder`, `webp::AnimDecoder`, `webp::BitstreamFeatures`
   (they parse bytes in C); `disallowed-methods`: `webp::Encoder::encode`,
   `webp::Encoder::encode_lossless` (they panic on error). A test fails on any
   `libwebp_sys` path in our source. The C decoder is compiled (the crate vendors all
   of libwebp) but unreachable by policy. The pinned `libwebp-sys` is covered by
   `cargo deny check advisories` and bumped when upstream libwebp ships a security fix.
8. Stored as BLOB; served from `/r/{n}/s/{id}` (project host) or
   `/p/{slug}/r/{n}/s/{id}` through `public_screenshots` bound to the project, with the
   visibility check, `Content-Type: image/webp`, `nosniff` and the handler's own
   `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'; sandbox`
   (§7). Admins view pending screenshots only through `/admin/p/{slug}/r/{n}/…`, never
   the public URL.
9. Rejecting a report as spam deletes its screenshots immediately.

### Public reads and JSON API

Paths are relative to the project base: `/p/{slug}` on the main host, `/` on the
project's `public_host`. In-app clients should use the main-host base
`/p/{slug}/api/v1`, which survives domain changes. Pages never call the JSON API (the
CSP has no `connect-src`): forms carry the solved PoW and post normally.

| Method and path | Input | Response |
|---|---|---|
| GET `/` | `?status=open\|in_progress\|closed\|fixed&cursor=…` | HTML list |
| GET, POST `/new` | form (multipart when screenshots are on) | form; 303 to a "received" page |
| GET `/r/{n}` | — | HTML detail |
| GET `/r/{n}/s/{id}` | — | screenshot (step 8 above) |
| GET `/api/v1/challenge` | `?purpose=submit\|email_code\|plus_one[&report=n]` | `{challenge}` |
| GET `/api/v1/reports` | `?status=…&cursor=…` | `{items: [PublicReport], next}` |
| GET `/api/v1/reports/{n}` | — | PublicReport + `body`, `body_html`, `screenshots`, `notes: [PublicNote]` |
| POST `/api/v1/reports` | `{title, body, pow, email_token?}` | 202 `{}` |
| POST `/api/v1/otp/send` | `{email, pow}` | `{verification_id}` |
| POST `/api/v1/otp/verify` | `{verification_id, email, code}` | `{email_token}` |
| POST `/api/v1/reports/{n}/plus-one` | `{pow}` | `{plus_one_count}` |

- `PublicReport`: `number`, `title`, `status`, `close_reason` (null unless closed),
  `plus_one_count` (null when +1 is off), `created_at`, `status_changed_at` (RFC 3339
  UTC), `url`. Detail adds `body` (markdown source), `body_html` (sanitized, as on the
  page), `screenshots` (URLs). `PublicNote`: `body`, `body_html`, `created_at`; no
  author.
- Errors: the HTTP status plus `{"error": "<code>", "message": "<text>"}`, with
  `"challenge"` added when the retry needs a fresh PoW.
- Every public list (HTML and API) has a hard page size of 50 with keyset pagination on
  a key that never changes while the report stays in that list: open, in progress and
  closed by `number` descending; Fixed by `status_changed_at` descending, then `number`.
  `cursor` is the last row's key, base64url; malformed → 400. List pages render titles
  only; markdown bodies render only on the detail page.
- JSON writes use the same PoW (documented algorithm + example client) and require
  `Content-Type: application/json`. Public GET reads allow CORS `*` without
  credentials; preflights for non-GET never get CORS headers, so CORS stays read-only.
  Admin never allows CORS.

## 7. HTTP security (every host, every response)

- **Headers**, outermost layer above the host routers, `/admin` included:
  CSP `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self';
  form-action 'self'; frame-ancestors 'none'; base-uri 'none'`,
  `Referrer-Policy: same-origin` (no-referrer would make browsers send `Origin: null`
  on same-origin POSTs; external links carry `rel=noreferrer`),
  `X-Frame-Options: DENY`, COOP same-origin, `X-Content-Type-Options: nosniff`, empty
  `Permissions-Policy`, `Strict-Transport-Security: max-age=31536000` (no
  includeSubDomains, no preload). The layer sets CSP and Cache-Control only where the
  handler set none; exactly two handlers set their own (screenshots, §6 step 8; the
  canonical 308), and a test asserts no other route does.
- **Cache-Control:** `no-store` on every `/admin` response, on login, invite, reset,
  setup/enrollment pages and on anything rendered with a session; `no-cache` on public
  HTML, `/api/v1` and screenshots; `max-age=3600` on the canonical 308; `public,
  max-age=31536000, immutable` only on hashed `/static`. Operators must not enable
  proxy/CDN caching for `/admin`, `/p`, `/api` or media.
- **Fetch-Metadata/Origin middleware** on every non-GET/HEAD request, admin and public:
  - `Sec-Fetch-Site` present and not `same-origin` → 403.
  - Else, `Origin` present must equal the expected origin exactly (`null` → 403).
  - Both absent → 403 for admin, login, reset, setup and HTML-form routes; allowed for
    JSON API writes (native clients send neither) and for the one-click unsubscribe
    POST on the main host, which its token authorizes (RFC 8058 senders are servers
    and send no cookies).
  - Expected origin comes from the matched host entry (`KOHAKU_BASE_URL`'s origin, or
    `https://{public_host}`), never from Host or forwarded headers.
- **Resource isolation** for `/admin` GETs: allow `Sec-Fetch-Site` `same-origin` and
  `none`; `same-site`/`cross-site` only with `Sec-Fetch-Mode: navigate`; requests
  without the header pass.
- Tracing spans record only axum `MatchedPath` and status, never the URI or query.

## 8. Admin surface

### Login

- One form: email, password, optional TOTP or recovery code. Wrong password, wrong
  code, or missing code for an enrolled account → the same generic error, one failure
  toward per-IP and per-account limits. No half-authenticated state: a new session
  token is issued only after all factors pass.
- An account that must use 2FA but is not enrolled (the admin, or a maintainer while
  2FA is required) never gets a session. After a correct password the form says
  "two-factor login is required; ask the admin for a setup link". The admin issues one
  with `kohaku admin reset-2fa` or Reset 2FA on the users page, which shows the link
  once (audited).
- Order: 64 KiB body cap → per-/64 and per-/48 login bucket → account lock check →
  argon2 semaphore (2 permits; one is reserved for requests carrying a valid
  `__Host-kohaku_device` cookie for the submitted account, checked first with SHA-256
  and an indexed lookup) acquired with a 2 s timeout (else 503) → re-check lock →
  argon2id (19 MiB, t=2, p=1; dummy hash for unknown emails) → TOTP.

### Lockout (device cookie)

- After a full login, set `__Host-kohaku_device` (`Secure; HttpOnly; SameSite=Strict;
  Path=/`, 1 year, main host only) with a random 256-bit token; store SHA-256 in
  `known_devices` (≤ 10 per user, oldest evicted). No signing key.
- Counters live in `users` (survive restarts), keyed on user_id after email
  normalization; unknown emails only hit IP limits. Failures count per account.
- Attempts **without** a valid device cookie for the account count in `failed_logins`;
  the 10th starts a 15 min lock (`locked_until`), and the count restarts at 0 when the
  lock ends.
- Attempts **with** one never hit that lock. They count in `device_failures` within a
  1 h window from `device_window_start`; past 20, they get the generic error until the
  window ends (at most 1 h, never a permanent lock).
- A successful login resets both counters. Password reset, password change and
  `reset-2fa` delete the user's `known_devices`.
- While locked: still run the dummy hash, return the generic error; never show
  "locked". Lockout mail at most once per account per 24 h (`lockout_mailed_at`).
- Strangers cannot lock the owner's browser out, and a login flood cannot take its
  reserved argon2 permit.
- Fallback: `kohaku admin unlock --email X` clears the lock and both counters.

### TOTP and recovery

- RFC 6238, SHA-1, 6 digits, ±1 step. Seeds derived (§5). Replay: accepted only via
  `UPDATE users SET totp_last_step=? WHERE id=? AND totp_last_step<?` on the writer.
  TOTP failures count toward login limits.
- 10 recovery codes of 16 base32 chars (80 bits), stored as SHA-256, consumed with
  `DELETE … WHERE user_id=? AND hash=?` checking rows affected.
- Required for admin; optional for maintainers unless the admin requires it.
- Enrollment shows a QR code of the `otpauth://totp/` URI (FreeOTP+ and other apps scan
  it) as inline SVG, since `data:` images are blocked by `img-src 'self'`, plus the base32
  secret as text. The code must be typed back before enrollment counts.
- Changing the password, disabling or re-enrolling 2FA and regenerating recovery codes
  need the current password plus a TOTP code if enrolled.
- Enabling 2FA revokes the user's other sessions. Turning on "require 2FA for
  maintainers" (`meta.require_2fa_maintainers`) revokes all sessions of non-enrolled
  maintainers; they log in again only after completing a setup link (§8 Login).

### Sessions and CSRF

- 256-bit token in `__Host-kohaku_session` (`Secure; HttpOnly; SameSite=Strict;
  Path=/`), DB stores SHA-256; 12 h idle / 7 d absolute; rotated on login. Password
  change, reset, or disable revokes all.
- Logout is a POST with the CSRF token; deletes the session row and expires the cookie.
- CSRF: per-session random token in every admin form + the §7 middleware.

### Token flows (invite, reset, setup)

- GET on a token URL only validates and renders a form. POST consumes atomically:
  `UPDATE tokens SET used_at=? WHERE hash=? AND purpose=? AND used_at IS NULL AND
  expires_at>?`, checking rows affected.
- **Setup** (1 h): created by `admin create`, `reset-2fa` (CLI or users page) and
  `admin rekey`; shown or printed, never mailed. The page sets the password and enrolls
  TOTP. After `admin create` the account has no password hash, so it cannot log in
  until done.
- **Invite** (48 h): admin enters email + grants. Acceptance only INSERTs a new user
  (unique email constraint), re-reads the invite in the same transaction and drops
  grants to projects that no longer exist. Passwords follow §4 Field limits. The
  users/invites page lists pending invites with Revoke (deletes row, audited).
- **Reset** (1 h): never creates a session, never clears or skips TOTP; revokes all the
  user's sessions and other reset tokens. Unknown or disabled users get the same
  response and the same work (always enqueue; their rows carry no token and the worker
  deletes them without sending). 3/hour per address (lowercased, `+tag` stripped, as
  for OTP) plus the public mail budget. Disabled for the admin role on the web; use
  `kohaku admin reset-password`.

### Moderation

- Single actions: approve, reject as spam, status changes, notes, hide/unhide, edit
  title/body/notes (audited), delete screenshots. Admin: hard delete.
- Bulk actions with confirmation: "reject selected as spam", "reject all pending from
  this source", "reject all pending".

### Pages

Dashboard (membership-filtered, quota warnings), project report list, report detail,
project settings (admin), users/invites (admin; Reset 2FA), audit log (admin), account
(password, 2FA, sessions).

## 9. Email

lettre to an external SMTP server, verified TLS required, plain text only. A `dev`
build sends nothing: it prints each mail in full to the terminal. Everything goes
through `outbox`; the worker sends security mail first and retries 1m → 5m → 30m → 2h,
then every 2 h, giving up
after 24 h or at the row's `expires_at` (OTP mail expires with its code, 10 min).
Subjects stripped of CR/LF, truncated; user text single-line, length-capped, marked as
quoted.

- Token-bearing mail (invite, reset, OTP) is deleted from `outbox` in the same
  transaction that marks it sent or given up; when delivery gives up, the token expires
  too. Setup links are never mailed.
- Maintainers: "new pending reports" mail with project name, count and admin-queue link
  only (no titles, bodies, emails or user URLs); coalesced to ≤ 1 per project per 10 min;
  per user per project opt-out (`project_members.notify`). Revoking a grant or opting
  out deletes that user's unsent rows for that project; disabling a user deletes all of
  that user's unsent rows.
- Verified reporters: approved, in progress, fixed, closed. `List-Unsubscribe` +
  `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (main-host URL); the unsubscribe
  page renders on GET and acts only on POST, which also accepts the header-less RFC 8058
  POST (§6 body caps, §7); unsubscribing deletes the `report_contacts` row.
- Security: lockout (≤ 1 per account per 24 h), password changed, invite accepted (to
  admin).

## 10. Operations

- `/healthz`. Logs to stdout never contain emails, IPs, tokens, query strings or report
  text. Rejected/abusive requests are logged as per-minute counters per reason, not per
  request.
- Retention job: spam text 30 days after `status_changed_at`; `report_contacts` 30 days
  after the report enters fixed, closed, hidden or spam (by `status_changed_at`);
  expired codes, sessions, tokens; outbox rows once sent or past the final attempt;
  audit entries older than 1 year; `*.tmp` files in `/data/` and `/data/backups/` older
  than 24 h (an interrupted backup or restore). Then `wal_checkpoint(TRUNCATE)`.
- Erasure (admin): deletes the report's `report_contacts` row and its outbox rows
  (`WHERE report_id=?`), then checkpoints. Earlier backups and volume snapshots still
  hold erased data (documented).
- Backup: `kohaku backup <file>` runs `VACUUM INTO` a temp name in the target directory,
  fsyncs, renames, mode 0600; refuses an existing target. `kohaku backup -` (the
  documented path: `docker compose exec -T kohaku kohaku backup - > file`) runs `VACUUM
  INTO` a `.tmp` file in `/data/backups/`, streams it to stdout and deletes it. With
  BLOB screenshots this one file is the whole backup.
- Restore: `kohaku restore <file>` or `restore -` (stdin) with the service stopped
  (`docker compose run --rm -T kohaku kohaku restore - < file`, so no host-owned file
  lands in the volume). It takes the instance lock (refuses while held), copies the
  source to a temp file in `/data` (uid 65532, 0600), fsyncs it and runs `PRAGMA
  integrity_check` on it; only then removes `-wal`/`-shm`, renames the temp file to
  `kohaku.db` and fsyncs `/data`. `restore --list` prints the files in `/data/backups/`
  (the image has no shell).
- Rollback of an upgrade: stop, `kohaku restore` the pre-migrate file, pin the previous
  exact tag.
- Proxy access logs contain token URLs and IPs; keep their retention short.

## 11. Development environment

- **Toolchain pin:** `rust-toolchain.toml` with an exact version (`channel = "1.NN.P"`,
  ≥ 1.89), musl targets, clippy, rustfmt; bumped deliberately. Used by rustup
  (sandbox/CI) and by the flake via rust-overlay.
- **Nix flake:** `devShells.default` (toolchain, just, watchexec, cargo-deny,
  cargo-zigbuild, zig, sqlite, `pkgs.openspec` 1.13.1 — no nodejs) and
  `packages.default` via `makeRustPlatform` with the rust-overlay toolchain from
  `fromRustupToolchainFile` (not nixpkgs' rustc). `flake.lock` pins OpenSpec. `.envrc`
  with `use flake`.
- **Non-Nix:** `rustup toolchain install` (reads `rust-toolchain.toml`); `cargo install
  --locked just@=X.Y.Z watchexec-cli@=X.Y.Z cargo-deny@=X.Y.Z` at the versions the
  flake provides, recorded in `docs/dependencies.md` and bumped with `flake.lock`;
  OpenSpec via `npm i -g @fission-ai/openspec@1.13.1`.
  OpenSpec never runs in CI or release jobs.
- **Live reload:** `just dev` runs `watchexec -r -- cargo run --features dev`. The `dev`
  feature serves CSS/JS from disk and a same-origin `/static/dev-reload.js` (~15 lines)
  that polls `/dev/boot-id` every second, keeps retrying while the server restarts and
  reloads when the id changes. Dev CSP = release CSP + `connect-src 'self'`, a cfg'd
  constant; no inline script. Template and Rust edits need an incremental rebuild
  (askama compiles templates).
- `#[cfg(all(feature = "dev", not(debug_assertions)))] compile_error!("dev feature in
  release build");`
- No mail server in development: a `dev` build prints every mail in full to the
  terminal instead of sending it (§9).

## 12. Testing

- **Unit:** PoW (purpose/project binding, difficulty), client IP (nginx-style forged
  prepended entry, mapped IPv4 peer, XFF from untrusted peer, two XFF headers, malformed
  entry; healthcheck from 127.0.0.1 and tls-ask from Caddy trigger no client-IP
  warning), rate limiter, TOTP (RFC 6238 vectors), markdown against an XSS corpus (incl.
  img, `tel:`, `//evil.example`, h1, deep nesting), JPEG SOF marker walk.
- **Integration:** axum `oneshot` against temp SQLite; `Mailer` trait with in-memory
  fake.
- **Route security matrix:** every route × {anon, non-member → 404, maintainer member on
  admin-only → 404, missing CSRF → 403}; every POST: `Origin: null` → 403,
  `Sec-Fetch-Site: cross-site` → 403; project-host form POST with
  `Origin: https://{public_host}` and no Sec-Fetch-Site → accepted; unsubscribe POST
  with a valid token, no Origin, no Sec-Fetch-Site and a multipart body → accepted and
  the `report_contacts` row deleted; non-enrolled maintainer with 2FA required and a
  correct password → no session. Member of A addressing B's child through A's slug →
  404; dashboard shows no ungranted rows. The test fails if a route is missing.
- **Host routing:** B's screenshot on A's host → 404; `/p/b` on A's host → 404; invite,
  reset, login paths on a project host → 404; tls-ask with Host `kohaku:8080` → 200
  configured / 404 unknown; reset with forged Host and X-Forwarded-Host still mails a
  `KOHAKU_BASE_URL` link; a host added by another process is served within 2 s.
- **Headers** (default-features build, `#[cfg(not(feature = "dev"))]`): exact CSP on a
  public page, on the admin report page showing a pending report and on a screenshot;
  Cache-Control per route class and on the canonical 308; no other route sets its own
  CSP or Cache-Control.
- **Concurrency:** 20 parallel requests where at most the limit may succeed: OTP verify,
  login lockout, recovery code, invite/reset token, PoW nonce, pending caps.
- **Timeouts:** a trickled multipart body is cut at the deadline with 408 and its permit
  is released.
- **Migrations:** for each version k, apply 1..k, insert one row per table, apply the
  rest, assert per-table row counts unchanged and `foreign_key_check` empty. No stored
  fixtures.
- **Screenshots:** 422 before decode for an 8000×5000 RGBA16 PNG (40 MP), a 4000×4000
  RGBA16 PNG (16 MP, 122 MiB) and a 24 MP progressive 4:4:4 JPEG. Succeed: a 24 MP
  RGBA8 PNG (the largest allowed buffer), a 24 MP baseline JPEG, a 10 MP progressive
  JPEG and the largest allowed lossy WebP with alpha; a 640×480 PNG keeps its
  dimensions. Peak memory per format is measured in a child process via `VmHWM`
  (cargo test threads share one process, so in-process RSS is meaningless): each job
  ≤ 160 MiB above baseline, two in parallel ≤ 320 MiB. Plus the libwebp ban test (§6
  step 7).
- **API:** one test asserts the JSON key set of `PublicReport`/`PublicNote`.
- **Logging:** a test asserts no IPs, emails, tokens or report text in logs.
- **Fuzz:** cargo-fuzz targets for markdown (`-timeout=2 -max_len=20480`), image
  pipeline, multipart (manual/nightly).
- **Manual (foundation acceptance):** one real-browser login POST and one real-browser
  report POST. Once by hand on a dual-stack host: IPv6 clients reach Kohaku with their
  real address (result recorded in the PR).

## 13. CI and release

- **CI (every push/PR, `pull_request` only):** `contents: read`, no secrets. Never
  `pull_request_target` or `workflow_run`. fmt, clippy `--all-targets --all-features -D
  warnings`, `cargo test` (default features; the gate) and `cargo test --all-features`,
  `cargo deny check` (advisories, licenses, bans, sources), static musl build, Docker
  build. Every cargo call `--locked`. Actions pinned by SHA, least-privilege
  `permissions:`.
- **cargo-deny:** `[bans] deny` openssl, openssl-sys, native-tls, aws-lc-rs,
  aws-lc-sys, rayon, rav1e, ravif; `[bans.build] allow-build-scripts` = the build-script
  crates at adoption (a new one fails CI); `[sources] unknown-registry = "deny"`,
  `unknown-git = "deny"`. No separate cargo-audit.
- **Updates:** Dependabot alerts on; `.github/dependabot.yml` for github-actions, docker
  and cargo, grouped, monthly. An advisory affecting a shipped crate → patch release.
- **Compose smoke test:** builds the image with the release Dockerfile from the build
  job's binaries; `docker compose config` on the shipped files; brings up `compose.yaml`
  on a fresh `kohaku-data` volume (proves `/data` ownership) with `KOHAKU_CADDY_CI`
  set; waits for healthy; `admin create` prints a link; `project create` with a host,
  then waits up to 5 s for the host map; copies Caddy's `root.crt` from `caddy-data`
  and uses `curl --cacert … --resolve` to assert: main host serves a public page, the
  project host gets a certificate and serves `/`, an unknown host's TLS handshake
  fails, `/p/{slug}` answers 308. Runs `backup -` through `docker compose exec -T` and
  `restore -` through `docker compose run --rm -T` once, as the README does (catches an
  ENTRYPOINT regression). Same Caddy tag as `compose.yaml`.
- **Release** (tag `[0-9]+.[0-9]+.[0-9]+`, no `v` prefix, must equal `Cargo.toml`):
  - `build` (matrix x86_64/aarch64 musl): `contents: read`, no secrets, no id-token,
    `persist-credentials: false`, no cache restore. `cargo deny check advisories`, then
    `cargo zigbuild --locked --release` (`cargo install --locked cargo-zigbuild@=X.Y.Z`,
    pinned zig); uploads binaries + SHA-256. A second job runs the aarch64 binary with
    `--version` on an `ubuntu-24.04-arm` runner.
  - `publish`: `needs` both, `environment: release`, `permissions: {contents: read,
    id-token: write}`. Verifies hashes, `docker login`, builds with a Dockerfile that
    only COPYs onto `gcr.io/distroless/static-debianNN:nonroot@sha256:…` (no RUN, no
    QEMU): the binary to `/usr/local/bin/kohaku`, and a directory holding only `.keep`
    to `/data` with `--chown=65532:65532 --chmod=0700` (a new named volume copies that
    ownership). `CMD ["kohaku", "serve"]`, no ENTRYPOINT. Pushes the multi-arch index
    by digest (`push-by-digest=true`) with provenance, signs it with `cosign sign --yes
    docker.io/dakes/kohaku@${DIGEST}` (`sigstore/cosign-installer@<sha>` with
    `cosign-release: v3.x.y`: bundle format, stored as an OCI referrer), then tags that
    digest with `docker buildx imagetools create`: `X.Y`, `X`, `latest`, and `X.Y.Z`
    last. No tag ever points at an unsigned digest, and a run that failed before
    `X.Y.Z` can be re-run.
  - The split protects credentials, not the binary's contents.
  - SBOM: `ARG BUILDKIT_SBOM_SCAN_CONTEXT=true` with `Cargo.lock` in the publish context
    so crates are listed; bundled SQLite and libwebp versions recorded in
    `docs/dependencies.md`. If the first attestation lists no crates, drop the SBOM
    attestation.
  - Release notes call out any `compose.yaml` or `Caddyfile` change.
- **Release protection** (setup in `docs/releasing.md`):
  - GitHub environment `release`: `DOCKERHUB_USERNAME`/`DOCKERHUB_TOKEN` only as
    environment secrets; deployment refs limited to tags `[0-9]*.[0-9]*.[0-9]*`; owner
    as required reviewer; only the publish job references it.
  - Tag ruleset for that pattern blocking update and deletion, no bypass actor.
  - Agent sandboxes hold no GitHub credentials; all pushes and release tags come from
    the owner's machine. Should an agent ever get GitHub access: a fine-grained PAT for
    `Dakes/Kohaku` only (Contents RW, Pull requests RW, Metadata R; no Workflows,
    Actions, Environments, Deployments, Administration or Secrets).
  - Docker Hub: Read & Write personal access token (never Delete), expiry ≤ 1 year.
    Personal tokens are account-wide on a personal namespace; if `dakes` holds other
    important repos, publish from a dedicated namespace. "Specific tags are immutable"
    with `^[0-9]+\.[0-9]+\.[0-9]+$`; `X.Y`, `X`, `latest` stay mutable by design.
- **Verify** (cosign ≥ 3.0; 2.x does not find bundle-format signatures): `cosign
  verify --certificate-oidc-issuer
  https://token.actions.githubusercontent.com --certificate-identity-regexp
  '^https://github\.com/Dakes/Kohaku/\.github/workflows/release\.yml@refs/tags/[0-9]+\.[0-9]+\.[0-9]+$'
  dakes/kohaku:X.Y.Z`.

## 14. Docker Compose (first-class deployment)

Shipped in the repo root; the README fetches the files from the release tag
(`https://raw.githubusercontent.com/Dakes/Kohaku/refs/tags/X.Y.Z/…`; the explicit
`refs/tags/` form cannot resolve to a branch of the same name). Security-relevant
behaviour (headers incl. HSTS, limits, validation) lives in the image, never in
`compose.yaml` or the `Caddyfile`, because `docker compose pull` never updates those
files.

- **`compose.yaml` kohaku:** not published on any host port. `read_only: true`,
  `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]`, `tmpfs: /tmp`, `user` from
  the image (nonroot, uid 65532), named volume `kohaku-data:/data` (a bind mount must be
  `chown -R 65532:65532` first), healthcheck `test: ["CMD", "kohaku", "healthcheck"]`,
  `mem_limit: 640m` (§3 bounds), `restart: unless-stopped`,
  `KOHAKU_BASE_URL: https://${KOHAKU_DOMAIN:?}`. Image pinned by major tag
  (`dakes/kohaku:1`).
- **caddy:** explicit tag (`caddy:2.N` or digest, same in the smoke test);
  `environment: KOHAKU_DOMAIN: ${KOHAKU_DOMAIN:?}` and `KOHAKU_CADDY_CI:
  ${KOHAKU_CADDY_CI:-}` (empty in production; CI sets `local_certs` +
  `skip_install_trust`); named volumes `caddy-data:/data`, `caddy-config:/config`
  (anonymous volumes lose certificates and the ACME account and can hit Let's Encrypt's
  5-per-week duplicate-certificate limit);
  `cap_drop: [ALL]` + `cap_add: [NET_BIND_SERVICE]` (the binary carries a file
  capability); `no-new-privileges`, `read_only: true`, `tmpfs: /tmp`, `mem_limit: 256m`.
  Only Caddy publishes ports (80/443).
- **`proxy` network:** private, dual stack, `enable_ipv6: true`; not `internal: true`
  (Kohaku needs SMTP and Caddy needs ACME). IPv4 subnet a /29 outside Docker's default
  pools and common LANs (e.g. `10.231.7.0/29`); a ULA /64 generated once and
  hard-coded. Caddy has fixed `ipv4_address` and `ipv6_address`;
  `KOHAKU_TRUSTED_PROXIES` lists exactly those two, in the kohaku `environment:` block
  (not `.env`), adjacent to the subnets: one set that changes together. IPv6 clients
  then reach Caddy via DNAT with their real address. Requires Docker Engine ≥ 27.
- **Logging:** `logging: driver: local` on both services (or json-file, max-size 10m,
  max-file 3).
- **`Caddyfile`:** global block with `{$KOHAKU_CADDY_CI}` and
  `on_demand_tls { ask http://kohaku:8080/.well-known/kohaku/tls-ask }`; the main-domain
  site `{$KOHAKU_DOMAIN}`; a catch-all `https://` site with `tls { on_demand }`. Public
  site blocks `respond /.well-known/kohaku/* 404`. Adding a project domain = DNS record
  first, then the admin UI; no Caddy reload. Remove hosts whose DNS no longer points
  here.
- **`.env.example`:** every required variable not fixed in `compose.yaml`
  (`KOHAKU_DOMAIN`, the one source of the main domain for both services; mail sender;
  SMTP server; the two secrets, left empty), no insecure placeholders.
- **`.env`** (gitignored, `chmod 600`): holds every setting, secrets included.
  `compose.yaml` passes `KOHAKU_SECRET` and `KOHAKU_SMTP_PASSWORD` as
  `${VAR:?message}`, so an empty one stops `docker compose up` with a hint. Anyone who
  can run `docker inspect` can read them; Docker access is root-equivalent anyway.
  `KOHAKU_SECRET` is backed up separately from database backups.
- Users with their own proxy delete `caddy`, attach `kohaku` to their proxy network,
  set `KOHAKU_TRUSTED_PROXIES` to the proxy's exact address and make the proxy set
  `X-Forwarded-For` (nginx `proxy_add_x_forwarded_for`, NixOS
  `recommendedProxySettings`). Kohaku's port must never be published beyond the proxy
  network.
- `docker compose down -v` deletes certificates and all data.

## 15. OpenSpec plan

One capability spec each, implemented as one change each, in order:

1. `foundation` — config (incl. `KOHAKU_SECRET` + keycheck, `KOHAKU_BASE_URL`
   validation, `KOHAKU_TRUSTED_PROXIES`), key registry, DB + migration runner (rules,
   downgrade guard, pre-migrate copy), pragmas and write rules, instance lock, graceful
   shutdown, `backup`/`restore` (file and stdin/stdout, `--list`), server skeleton, host
   map + two routers + internal endpoints, URL builder, client IP + rate limiter (incl. `read` class), header and
   Cache-Control layer, Fetch-Metadata/Origin middleware, per-route body caps,
   `audit_log` + `audit()` helper, outbox worker + public mail budget, retention job
   skeleton, assets, Nix flake, dev loop, Docker, Compose (+ smoke test, extended as
   later changes add CLI commands), CI, release + `release` environment
2. `admin-auth` — login, sessions, CSRF, logout, device cookie + lockout +
   `admin unlock`, TOTP (derived seeds, replay, recovery codes), re-auth, `tokens` table,
   password reset + `admin reset-password`
3. `projects` — CRUD, per-project flags (incl. `features_enabled`), custom domain +
   tls-ask answers, canonical redirect, `project create`, `RequireAdmin`
4. `users-and-invites` — setup-link bootstrap (`admin create`, `admin reset-2fa`,
   `admin rekey`, which all print setup links), revocable invites with grants, grants,
   required 2FA for maintainers, `ProjectAccess<Cap>` (after `projects`: grants reference
   projects, and tables reference only earlier migrations)
5. `report-submission` — form, check order, PoW, honeypot, per-source cap + backstop,
   markdown pipeline, JSON API writes, report kind
6. `moderation` — lifecycle + transition table incl. `hidden`, bulk actions, audited
   edits, admin hard delete, notes, visibility views, public pages + API reads with
   pagination, Fixed section, bugs and feature requests told apart

Feature requests (owner decisions 2026-09-24): a finished feature uses the `fixed`
status, labelled "Implemented" (bugs: "Fixed"); the section is "Fixed / Implemented", or
"Fixed" in a project that takes only bugs. Lists and the API filter by kind, and every row
shows its kind at a glance. Maintainers may change a report's kind (audited). +1 also
sorts open lists ("most wanted"). Screenshots apply to both kinds.

After 7 the tracker is usable; 8–11 add features.
