# Kohaku — Design

Date: 2026-09-23 · Status: validated in brainstorming, pending security review

## 1. What it is

A tiny, self-hosted bug report inbox. The admin creates projects; each project gets a
public report form, a public list of reports (open / in progress / closed) with a
permanent **Fixed** section, and a JSON API. Reporters never register. New reports are
private until a maintainer approves them (moderated public). Aimed at small developers
(apps, SaaS, game mods). Open source.

**Principles:** security first (public, unauthenticated input), minimal resources,
minimal dependencies, boring and auditable code.

**Non-goals:** boards, sprints, assignees, labels, public comment threads, reporter
accounts, HTML email, SPA frontend.

## 2. Decisions log

| Topic | Decision | Why |
|---|---|---|
| Backend | Rust stable, axum + tokio, single static musl binary | Fixed by owner |
| Frontend | Server-rendered askama templates, one CSS, small vanilla JS + PoW worker; no React | No npm supply chain, no JSON admin API, works without JS, strict CSP |
| Visibility | Moderated public: `pending` is private until approved | Spam never goes public |
| Anti-spam | PoW + honeypot + rate limits always; email OTP per project (optional) | Anonymous by default, stronger gate where wanted |
| Admins | One admin (CLI bootstrap) + maintainers invited by email with per-project grants | Owner request |
| Report content | Markdown (no raw HTML, no images), screenshots per project (off by default) | Owner request |
| Screenshots | Decode in pure Rust, re-encode lossy WebP via libwebp | Untrusted bytes never reach C; small files |
| Interaction | Maintainer public notes + "me too" counter (per project) | No comment threads |
| 2FA | TOTP in v1: required for admin, optional for maintainers (admin can require) | Admin account sees everything |
| TLS | Behind a reverse proxy; trusted-proxy CIDRs for client IP | Fewer crates |
| Spec process | OpenSpec (spec-driven schema), one change per capability | Owner request |
| Distribution | Docker Hub image built by GitHub Actions on bare semver tag (`1.0.0`) | Owner request |
| Dev env | Nix flake (devShell + package) and rustup both supported | Owner on NixOS |
| Domains | Optional custom domain per project on one instance; admin only on the main domain | Owner request; isolates user content from admin origin |
| Deployment | Docker Compose is the first-class path: shipped `compose.yaml` + Caddy, CI-tested | Owner request |

## 3. Architecture

```
reverse proxy (TLS) ──► kohaku (axum, HTTP :8080, non-root, distroless static)
                          ├── host router     Host → main domain | project domain | 421
                          ├── public routes   main: /p/{slug}/…   project domain: /…
                          ├── public API      main: /p/{slug}/api/v1/…   project domain: /api/v1/…
                          ├── admin routes    /admin/… (main domain only) session auth + CSRF
                          ├── tls-ask         /.well-known/kohaku/tls-ask?domain=…  (for Caddy)
                          ├── static assets   /static/…       embedded, hashed names, immutable
                          ├── SQLite (WAL)    /data/kohaku.db
                          ├── screenshots     /data/media/{random-id}.webp
                          └── background      mail outbox, notification coalescing, retention
```

- DB access: rusqlite on a small `spawn_blocking` pool — one writer connection, N readers.
  `STRICT` tables, `foreign_keys=ON`, migrations via `PRAGMA user_version`.
- Config: env vars (primary, Compose-friendly), optional TOML file. Every secret also
  accepts a `*_FILE` variant (e.g. `KOHAKU_SMTP_PASSWORD_FILE`) for Docker/Compose
  secrets, so secrets never appear in `docker inspect`. Never configurable via web UI.
- CLI: `kohaku serve | healthcheck | admin create | admin reset-2fa | backup <file>`
  (`VACUUM INTO`). `healthcheck` probes `/healthz` on localhost, because the distroless
  image has no shell or curl for a Compose `healthcheck:`.
- Our code: `#![forbid(unsafe_code)]`.

### Crates (each must be justified in `docs/dependencies.md`)

axum, tokio, tower-http (limits, timeouts, headers) · rusqlite (bundled) · askama ·
argon2, hmac, sha2, sha1 (TOTP), getrandom · lettre (rustls only) · pulldown-cmark,
ammonia · image (png/jpeg/webp decoders only), webp (libwebp encoder) · serde,
serde_json, toml · tracing, tracing-subscriber. Dev-only: tower-livereload (behind
`dev` feature), proptest if needed.

## 4. Data model

- `users` — email, argon2id hash, role `admin|maintainer`, disabled, totp secret (encrypted? see review), recovery code hashes
- `project_members` — maintainer ↔ project grants
- `invites` — SHA-256(token), email, grants, expiry 48h, single use
- `sessions` — SHA-256(token), user, created, last_seen, expiry
- `projects` — slug, name, `public_host` (optional, unique), `screenshots_enabled` (default off), `require_email`, `me_too_enabled`
- `reports` — per-project number, title, markdown source, status, close_reason,
  reporter_email (verified, optional, never public), notify flag, me_too_count, timestamps
- `report_notes` — public maintainer notes (markdown)
- `screenshots` — random 128-bit id, report, dims, bytes
- `me_too` — (report_id, voter_hash) unique; voter_hash = HMAC(rotating key, IP prefix)
- `email_codes` — hash(email), hash(code), attempts, expiry
- `outbox` — queued mail with attempts/backoff
- `audit_log` — actor, action, target, time

### Lifecycle

```
submit → pending (private) ──reject──► spam (private, purged after 30 days)
            │ approve
            ▼
          open ──► in_progress ──► fixed (permanent "Fixed" section)
            └────────┴──────────► closed (won't fix / duplicate; public with reason)
```

Public visibility is one predicate (`status NOT IN ('pending','spam')`) used by every
public query and the screenshot handler.

### Access control

A single `ProjectAccess` extractor loads the project and checks role/membership for every
project-scoped admin route; non-members get **404**. Maintainers: moderate, status,
notes, delete screenshots on granted projects. Admin only: projects, settings, invites,
grants, audit log.

## 5. Public surface

### Domains and host routing

- `KOHAKU_BASE_URL` (required) is the **main domain**: admin area plus projects without
  a custom domain at `/p/{slug}`.
- A project may have a `public_host` (e.g. `bugs.pocetude.com`), set by the admin. On
  that host the project is served at the root (`/`, `/new`, `/r/{n}`, `/api/v1/…`), and
  `/p/{slug}` on the main domain answers **308** to it (one canonical URL per report).
- The host router resolves every request's `Host` (port stripped, lowercased) against
  the main host and the `public_host` set (cached in memory, refreshed on change).
  Unknown hosts get **421 Misdirected Request**; `/admin` on a project host gets 404.
- Absolute URLs (emails, redirects, canonical links) come from the URL builder using
  configured hosts only, **never from the request `Host`**.
- `public_host` validation: lowercase ASCII DNS name (IDN must be entered as punycode),
  ≤ 253 chars, at least one dot, not an IP literal, not equal to the main host.
- `tls-ask` answers 200 only for configured `public_host` values, so Caddy on-demand
  TLS never requests certificates for arbitrary domains. It reveals only whether a
  domain is configured, which DNS already reveals.
- Session cookies are `__Host-` on the main host only, so script on a project domain
  (should an XSS ever slip through) cannot reach admin sessions. Operators are advised
  to use a dedicated admin domain even for single-project installs.
- PoW challenges bind the project, so a solution cannot be replayed across domains.

### Submission

Submission: `GET /p/{slug}/new` returns the form + an HMAC-signed PoW challenge
(project, nonce, difficulty, expiry). A web worker finds a counter such that
SHA-256(challenge‖counter) has N leading zero bits. On POST, checks run cheapest first:
body size cap → honeypot → IP rate limit → PoW valid and unused → email token (if
required) → field validation → screenshots (only if enabled) → insert `pending` →
queue maintainer notification.

- Rate limits: in-memory token buckets per client IP (IPv6 /64), per action class.
  `X-Forwarded-For` honoured only from trusted proxy CIDRs.
- Backstop: max pending reports per project (default 500) → form reports unavailable.
- Email OTP: sending a code costs a PoW; 3 codes/address/hour; mail body is the code
  only (no user text); 6 digits, 10 min, 5 attempts; only HMACs stored. Success yields a
  signed 30-min "verified email" token.
- Me too: lighter PoW; one per voter hash per report.
- Markdown: pulldown-cmark, raw HTML → text, images removed, links http/https/mailto
  only with `rel="nofollow noopener noreferrer ugc"`, then ammonia allowlist. Source ≤ 20 KB.
- Screenshots: ≤ 3 per report, ≤ 8 MB each, ≤ 40 MP decoded, magic bytes PNG/JPEG/WebP,
  pure-Rust decode with limits, resize to 2560 px longest edge, lossy WebP q80, 2
  concurrent encodes, metadata dropped, originals never stored. Served with `nosniff`,
  sandbox CSP, visibility check.
- JSON API `/api/v1`: same PoW (documented algorithm + example client). Public reads
  allow CORS `*` without credentials; admin never allows CORS.
- Headers: CSP `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self';
  form-action 'self'; frame-ancestors 'none'; base-uri 'none'`, `Referrer-Policy:
  no-referrer`, `X-Frame-Options: DENY`, COOP same-origin, empty `Permissions-Policy`.

## 6. Admin surface

- Login: argon2id (19 MiB, t=2, p=1), dummy hash for unknown emails, argon2 behind a
  semaphore. Limits per IP and per account; 10 failures → 15 min lock + email.
- TOTP (RFC 6238, SHA-1, 6 digits, ±1 step, replay of last used step rejected) + 10
  one-time recovery codes. Required for admin; optional for maintainers unless required
  by admin. `kohaku admin reset-2fa` for lockout recovery.
- Sessions: 256-bit token in `__Host-kohaku_session` (`Secure; HttpOnly;
  SameSite=Strict; Path=/`), DB stores SHA-256; 12 h idle / 7 d absolute; rotate on
  login; password change or disable revokes all.
- CSRF: SameSite=Strict + per-session HMAC token in every form + `Origin` check.
- Invites: admin enters email + grants → single-use 48 h link → maintainer sets password
  (≥ 12 chars, NIST 800-63B). Password reset: same, 1 h, identical response either way.
- Pages: dashboard, project report list, report detail, project settings (admin),
  users/invites (admin), audit log (admin), account (password, 2FA, sessions).

## 7. Email

lettre, TLS required, plain text only. Everything goes through `outbox`; worker retries
1m → 5m → 30m → 2h, gives up after 24 h. Subjects stripped of CR/LF, truncated.

- Maintainers: new pending reports, coalesced to ≤ 1 mail per project per 10 min; per
  user per project opt-out.
- Verified reporters: approved, in progress, fixed, closed; signed unsubscribe link +
  `List-Unsubscribe`.
- Security: lockout, password changed, invite accepted (to admin).

## 8. Development environment

- **Toolchain pin:** `rust-toolchain.toml` (stable channel, musl targets, clippy,
  rustfmt). Used by rustup (sandbox/CI) and by the Nix flake via rust-overlay, so
  every environment uses the same compiler.
- **Nix flake:** `devShells.default` (toolchain, bacon/watchexec, cargo-audit,
  cargo-deny, cargo-zigbuild, zig, sqlite, mailpit, nodejs for OpenSpec) and
  `packages.default` (`rustPlatform.buildRustPackage` from `Cargo.lock`). `.envrc` with
  `use flake` for direnv users.
- **Live reload:** `just dev` / `bacon dev` runs `watchexec -r` on `cargo run --features dev`.
  The `dev` feature (never in release builds) adds `tower-livereload` to refresh the
  browser after restart and serves CSS/JS from disk instead of the embedded copy, so
  asset edits are instant. Template and Rust edits need an incremental rebuild (a few
  seconds) because askama compiles templates. `listenfd` keeps the socket open across
  restarts so the browser never sees "connection refused".
- `compose.dev.yaml` with Mailpit for inspecting mail.

## 9. Testing

- Unit: PoW, proxy IP extraction, rate limiter, TOTP (RFC 6238 vectors), markdown
  renderer against an XSS corpus.
- Integration: axum `oneshot` against temp SQLite; `Mailer` trait with in-memory fake.
- Route security matrix: every admin route × {anon, non-member → 404, missing CSRF →
  403, allowed}; test fails if a route is missing from the matrix.
- Fuzz: cargo-fuzz targets for markdown, image pipeline, multipart (manual/nightly).

## 10. CI and release

- **CI (every push/PR):** fmt, clippy `-D warnings`, test, cargo audit, cargo deny
  (licenses, bans, sources = crates.io only), static musl build, Docker build. Actions
  pinned by SHA, least-privilege `permissions:`. **Compose smoke test:** build the
  image, `docker compose config` on the shipped files, bring up `compose.yaml`
  (with a CI override for the image tag and a local CA), wait for healthy, run
  `admin create` non-interactively, fetch a public page through Caddy.
- **Release (on tag matching `[0-9]+.[0-9]+.[0-9]+`, no `v` prefix; must equal the `Cargo.toml` version):** cross-compile `x86_64` and `aarch64` musl binaries with
  cargo-zigbuild (no QEMU), build a multi-arch image on `gcr.io/distroless/static:nonroot`,
  push to Docker Hub as `X.Y.Z`, `X.Y`, `X`, `latest`; attach SBOM and provenance
  attestations; sign with cosign keyless (GitHub OIDC). Secrets:
  `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN` (access token, write scope only for the repo).

## 10a. Docker Compose (first-class deployment)

Shipped in the repo root and documented in the README as the primary install path:

- `compose.yaml`: `kohaku` + `caddy` (automatic HTTPS). Kohaku is not published on
  any host port; only Caddy is (80/443). The two share a `proxy` network with a
  **fixed subnet**, so `KOHAKU_TRUSTED_PROXIES` is an exact, explicit CIDR instead of
  "trust the Docker bridge". Kohaku service hardening: `read_only: true`,
  `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]`, `tmpfs: /tmp`,
  `user` from the image (nonroot), named volume `kohaku-data:/data`, healthcheck via
  `kohaku healthcheck`, memory limit, `restart: unless-stopped`.
- `Caddyfile`: the main domain from `.env`, plus a catch-all `https://` site with
  `tls { on_demand }` and a global `on_demand_tls { ask http://kohaku:8080/.well-known/kohaku/tls-ask }`.
  Adding a project domain is: DNS record + set it in the admin UI. No Caddy reload.
- `.env.example`: every required variable, no insecure placeholders (startup fails on
  unchanged example values like `bugs.example.com`).
- `secrets/` convention for `smtp_password` via Compose `secrets:` (gitignored).
- Image pinned by major tag (`dakes/kohaku:1`), so `docker compose pull` gets fixes
  but never a breaking release.
- Users who already run a proxy delete the `caddy` service and attach `kohaku` to
  their proxy's network; the README shows the Traefik/nginx variant in a few lines.

## 11. Operations

`/healthz`; logs to stdout never contain emails, IPs, tokens or report text. Retention
jobs purge spam (30 d), expired codes/sessions/invites, sent outbox rows. Admin can
erase a reporter email from a report.

## 12. OpenSpec plan

One capability spec each, implemented as one change each, in order:

1. `foundation` — config, DB/migrations, server skeleton, host router + URL builder, headers, assets, Nix flake, dev loop, Docker, Compose (+ smoke test), CI, release
2. `admin-auth` — login, sessions, CSRF, lockout, TOTP, password reset
3. `users-and-invites` — CLI bootstrap, invites, grants
4. `projects` — CRUD, per-project flags, custom domain + tls-ask
5. `report-submission` — form, PoW, honeypot, rate limits, markdown, JSON API
6. `moderation` — lifecycle, notes, public pages, Fixed section
7. `notifications` — outbox, coalescing, unsubscribe
8. `reporter-verification` — email OTP
9. `screenshots`
10. `me-too`
11. `audit-log`

After 7 the tracker is usable; 8–11 add features.
