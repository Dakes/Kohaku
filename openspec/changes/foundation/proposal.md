# Proposal

## Why

Kohaku has a validated, security-reviewed design
(`docs/plans/2026-09-23-kohaku-design.md`) but no code. Every later change (auth,
projects, reports, moderation, mail) relies on the same platform: configuration that
fails closed, a safe SQLite layer with migrations, host routing, the HTTP security
layer, abuse limits, the mail outbox, the audit trail, and a build, release and
deployment pipeline that cannot publish an image without the owner's approval. Building
these first, once, means no feature change can skip them.

## What Changes

- New `kohaku` binary (single static musl build) with the CLI `serve | healthcheck |
  backup | restore`.
- Configuration from environment variables only, secrets included (one `.env` for Compose);
  required settings have no defaults; startup fails on missing, malformed or example values
  and on an instance secret that does not match the database.
- SQLite database with a migration runner (downgrade guard, pre-migration copy,
  foreign-key check), an instance lock, graceful shutdown, streaming backup and
  integrity-checked restore, and a retention job skeleton.
- Host routing: main host, project hosts (none exist until `projects`), 421 for unknown
  hosts, `/healthz` before routing, and a URL builder that never uses request headers.
- HTTP security layer on every response: CSP and security headers, HSTS,
  Cache-Control classes, Fetch-Metadata/Origin checks on every non-GET request,
  per-route body caps and request deadlines.
- Abuse limits: client-IP resolution behind trusted proxies, rate-limit classes,
  bounded concurrency with 503 when full, and the shared budget for unauthenticated mail.
- Mail outbox: plain-text mail through an external SMTP server with verified TLS, retries,
  priorities, expiry, and deletion of token-bearing rows once they are sent or given up. `dev`
  builds print mail to the terminal instead; no mail server is needed in development.
- Audit log: an append-only record of state changes, ids only, kept 1 year.
- Minimal public pages: an instance landing page on the main host and embedded static
  assets, so the pipeline has something real to serve and test.
- Distribution: Nix flake and rustup dev setups, `just dev` live reload behind a `dev`
  feature that cannot reach a release build, Dockerfile, shipped `docker-compose.yml` +
  `Caddyfile` + `.env.example`, CI, the Compose smoke test, and the gated, signed release
  workflow.

## Capabilities

### New Capabilities

- `configuration`: how Kohaku is configured and when it refuses to start.
- `data-storage`: database lifecycle: migrations, atomic limit enforcement, instance
  lock, backup, restore, retention.
- `host-routing`: which host serves what, unknown-host handling, internal endpoints,
  absolute URL generation.
- `http-security`: response headers, caching, cross-site request rejection, body caps,
  request deadlines.
- `request-limits`: client-IP resolution, rate-limit classes, concurrency bounds, public
  mail budget.
- `mail-outbox`: queued, retried, plain-text outbound mail and what it may contain.
- `audit-log`: what is recorded about state changes and for how long.
- `operations`: health checks, logging privacy, graceful shutdown, CLI behaviour.
- `distribution`: the published image, the Compose deployment, and the release gate.

### Modified Capabilities

None (first change).

## Non-goals

- Any user-facing feature: accounts, login, projects, reports, moderation, notifications
  about reports, screenshots, +1. Those arrive in changes 2–11 of the design's
  OpenSpec plan.
- TOML config file support (see design.md: env-only for v1).
- An admin UI. `/admin` returns 404 until `admin-auth`.
- Answering Caddy's tls-ask with configured domains (the `projects` change; until then
  it answers 404 for every domain).

## Security considerations

- **Attacker: anonymous internet client.** Surfaces: every route on every host. The
  header layer, Fetch-Metadata/Origin middleware, body caps, deadlines, rate limits and
  concurrency bounds are in place before the first feature route exists, and the route
  security matrix test makes a new route without them fail CI.
- **Attacker: forged headers** (`Host`, `X-Forwarded-For`, `X-Forwarded-Host`). Unknown
  hosts get 421; forwarded client IPs are trusted only from configured proxy addresses;
  absolute URLs come only from configuration.
- **Attacker: resource exhaustion.** Every queue and map has a fixed bound; overflow
  answers 503 or 429 instead of growing memory.
- **Attacker: holder of a leaked backup.** The instance secret lives outside the
  database; the keycheck prevents running a database with the wrong secret.
- **Attacker: local user on the Docker host.** Secrets sit in `.env` (mode 0600) and the
  container environment; only root and Docker users, who control the host anyway, can read
  them. Kohaku never logs or prints them.
- **Attacker: compromised dependency or dev machine.** cargo-deny bans and source
  rules, locked builds, no secrets in CI, publish gated by the `release` environment
  with owner approval, images signed by digest, version tags immutable.
- **Operator mistakes.** No insecure defaults; example values refuse to start; the
  `dev` feature cannot compile into a release build.

## Impact

New repository layout (`Cargo.toml`, `src/`, `templates/`, `static/`, `migrations/`,
`flake.nix`, `Dockerfile`, `docker-compose.yml`, `Caddyfile`,
`.env.example`, `.github/`, `deny.toml`, `clippy.toml`, `rust-toolchain.toml`,
`docs/dependencies.md`).

New dependencies (each also justified in `docs/dependencies.md`):

- `axum`: HTTP routing and extractors; the design's chosen web framework.
- `tokio`: async runtime required by axum.
- `hyper-util`: server builder for the header read timeout axum's `serve` lacks
  (already a transitive dependency of axum).
- `tower` (`util`): `Service`/`Layer`, `service_fn` and `oneshot` for the host dispatcher and
  the header layer, which wrap the routers outside axum routing (already compiled into the
  tree by axum).
- `tower-http`: body limit, timeout and trace layers instead of hand-written ones.
- `rusqlite` (`bundled`): SQLite with a pinned, statically linked library.
- `askama`: compile-time, auto-escaping HTML templates.
- `hmac`, `sha2`: MACs and hashes for the key registry, keycheck and token hashes.
- `getrandom`: OS randomness for per-boot keys and tokens.
- `base64`: instance-secret decoding (already transitive through lettre).
- `lettre` (rustls + ring + webpki-roots, no native-tls): SMTP with TLS.
- `idna_adapter` (`~1.1`): selects the small unicode-rs IDNA backend for lettre's `idna`.
- `serde`, `serde_json`: JSON error bodies and the healthz response.
- `tracing`, `tracing-subscriber`: structured logs to stderr/stdout.
- `rustls` (dev-dependency only, already in the tree through lettre): the local TLS peer that
  tests SMTP certificate verification without a mail server.
