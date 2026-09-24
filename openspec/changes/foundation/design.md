# Design

## Context

`docs/plans/2026-09-23-kohaku-design.md` ("§N") stays authoritative, the specs hold every
requirement and number, proposal.md says why. This document adds only mechanisms,
ambiguities resolved while drafting, and deviations (D31). Beyond the design:

- Every seam (route entry, migration, key purpose, rate class, permit, mail kind, audit
  action, CLI command, retention step) is built and tested now, synthetically where unused.
- `forbid(unsafe_code)` on Rust 2024 bans `set_var`: configuration uses an injected source.
- SQLite (`foreign_keys=ON`) refuses inserts into a table whose FK parent table is missing,
  even with a NULL FK (verified): tables reference only earlier migrations.

Versions at writing: axum 0.8.9, tokio 1.53, hyper-util 0.1.20, tower 0.5.3, tower-http
0.7.1, rusqlite 0.40, askama 0.16, hmac 0.13, sha2 0.11, getrandom 0.4, base64 0.23, lettre
0.11.23, idna_adapter 1.1.0; ~105 crates, no h2, ICU4X, openssl, native-tls, aws-lc, rayon.

## Goals / Non-Goals

**Goals:** later changes only add (routes, migrations, key purposes, rate classes, permits,
mail kinds, audit actions, commands, retention steps, settings), never editing the pipeline,
migration runner or `release.yml`; routes exist only in the route table, which the matrix
walks exhaustively; every §3 bound holds from day one; the image is the binary CI tested.

**Non-Goals:** project hosts, host-map refresh, watcher connection, 308 (`projects`);
sessions, CSRF, non-public guards (`admin-auth`, `users-and-invites`); other rate classes
and permits, PoW used-set, multipart body class, the other §3 tunables, account recipients,
outbox links, RFC 3339 (their changes). Not here: HTTP/2, TLS termination, per-request access
logs, a configurable port or data directory.

## Decisions

**D1. Dependencies and features.** All `default-features = false`, with exactly: axum `http1`,
`tokio`, `matched-path`; tokio `rt-multi-thread`, `net`, `time`, `sync`, `signal`, `macros`;
hyper-util `tokio`, `server`, `http1`, `service`, `server-graceful`; tower `util`; tower-http
`limit`, `timeout`, `trace`; rusqlite `bundled`; askama `derive`, `std`; base64 `std`; lettre
`smtp-transport`, `builder`, `tokio1-rustls`, `ring`, `webpki-roots`; serde `std`, `derive`;
serde_json and tracing `std`; tracing-subscriber `fmt`, `std`; hmac, sha2, getrandom,
idna_adapter none. Notably off: hyper-util `server-auto` (h2), askama `config` (TOML parser),
lettre `pool` (one connection at a time), tracing `attributes` (proc-macro), tracing-subscriber
`env-filter` (regex; `RUST_LOG` must not change logging); axum `json`/`form`/`query` wait for a
route parsing such input. No direct `hyper`; the only dev-dependency is `rustls` for D18's TLS
peer (already in the tree through lettre, same version); `--locked` everywhere;
`docs/dependencies.md` records the unified features and the bundled SQLite version, reviewed on
every bump. `tower` is direct because the dispatcher and header layer wrap the routers outside
axum and need `Service`/`Layer` (axum already compiles it in); one Router for all hosts instead
would make `/` one handler branching on Host.

**D2. Not dependencies.** clap → a `match` over `args_os()`; toml → D31.1; chrono, time →
INTEGER unix seconds; ipnet → ~40 lines of CIDR code; url → a strict `https://host[:port]`
parser (url normalizes, so its output would need re-checking); rand, tempfile, thiserror,
anyhow, once_cell, arc-swap, dashmap, parking_lot, tokio-util → getrandom and std; an HTTP
client → `std::net::TcpStream`; rust-embed, include_dir, mime_guess → `include_bytes!`.

**D3. Toolchain, lint and supply-chain files.** `rust-toolchain.toml` as §11 with `profile =
"minimal"`; `Cargo.toml`: edition 2024, `rust-version` = that pin, `unsafe_code = "forbid"`
lint plus crate-root attributes, `[features] dev = []`, explicit `panic = "unwind"`.

- `deny.toml` = §13 plus `[graph] targets` (both musl targets, x86_64-unknown-linux-gnu),
  `[advisories]` denying vulnerabilities, `[licenses] allow` and `allow-build-scripts` =
  exactly the tree's at adoption (7 licenses incl. CDLA-Permissive-2.0, 13 scripts).
- `clippy.toml` `disallowed-methods` with reasons: axum `Router::{route, route_service,
  nest, nest_service, merge, fallback, fallback_service}` (only `routing::build`);
  `std::env::{var, var_os, vars, vars_os}` (only `config.rs`); lettre
  `AsyncSmtpTransport::{builder_dangerous, unencrypted_localhost}` and
  `TlsParametersBuilder::{dangerous_accept_invalid_certs,
  dangerous_accept_invalid_hostnames}`; `TraceLayer::new_for_http` (records URIs).
- `tests/source_scan.rs` fails on an exemption outside those two modules, on `Tls::None`
  or `Tls::Opportunistic` in `src/`, since clippy cannot ban enum variants, and on any SMTP
  trust anchors in `src/` other than `SmtpRoots::compiled()` (D18).

**D4. Package and module layout.** A library plus a thin `main.rs` returning `cli::main()`'s
exit code, so integration tests reach internals and caller-less seams (mail budget,
public-write permit, per-boot keys, enqueue) stay `pub` without tripping `dead_code`; each is
tested against its spec now, and its first caller owns any adjustment.

```
src/        lib cli config keys time audit jobs logging pages assets dev test_support
  db/       mod (connections, pragmas), migrate, lock, backup (backup, restore, list), paths
  routing/  table, build, hosts (map, normalization, dispatch), urls, internal
  http/     server, headers, fetch (origin check, admin isolation), body, errors
  limits/   client_ip, rate, permits, mail_budget
  mail/     mod (Mailer, message rules), smtp (SmtpRoots), dev, outbox (enqueue, worker, give-up)
templates/  base, landing, error          static/  kohaku.css, dev-reload.js
migrations/ 0001_foundation.sql
tests/      route_matrix hosts headers fetch limits migrations backup_restore outbox smtp_tls
            logging source_scan support/ (test_support.rs, test config, raw-socket client)
            fixtures/smtp/ (certificates, generate.sh)
```

**D5. Configuration.** `Config::load(command, lookup)`: the binary passes the environment,
tests pass maps; `serve` loads everything, `backup` only `KOHAKU_SECRET`, `restore` and
`healthcheck` nothing. SMTP settings are `KOHAKU_SMTP_HOST`, `_PORT`, `_TLS`
(`implicit`|`starttls`), `_USERNAME`, `_PASSWORD` and `_FROM`; the only optional one is
`KOHAKU_PUBLIC_MAIL_PER_HOUR`. A `dev` build reads only `_FROM` and refuses the other five when
set (D18, D23). Secrets are plain variables like the rest (owner decision, design §3); the
environment is never logged or printed, and errors name variables, never values. Values are
taken exactly (no trimming or case folding), all problems reported in one run; secret types
implement neither `Debug` nor `Display`. One DNS-name validator serves the
base-URL host, SMTP host, sender domain and host-map entries; the sender has its own strict
validator (lettre's `Address` accepts more). `config.rs` holds `.env.example`'s placeholders
(and the base URL Compose derives from them), and a test asserts each is refused. `serve`
logs the effective value of each numeric setting, exposing misspelled names.

**D6. Key registry.** The instance secret (≥ 32 bytes) is only an HMAC-SHA256 key. One function
builds every MAC input (purpose label, then each field prefixed by its u32 big-endian length)
and one verify function wraps `verify_slice`; per-boot keys use both. `Purpose` maps to
`kohaku/…` labels, foundation defines only `Keycheck` (later purposes, TOTP and source, update
§5's inventory), and a test checks labels are unique and prefixed. `PerBootKey` (32 getrandom
bytes, no `Debug`, no serialization) is defined and tested now; a random-source failure stops
startup. Keycheck order: an existing database is checked before anything is written (copy,
migration, row, listener); a new one gets its keycheck in migration 1's transaction; one found
without a keycheck is completed by `serve` and refused by `backup`.

**D7. Database access.** One writer (`secure_delete=ON`) and four readers (`query_only=ON`,
so a write through a reader fails), opened at startup. `db.write`/`db.read` take a tokio
semaphore permit (1/4) and move it into `spawn_blocking`: bursts wait asynchronously instead
of parking blocking threads, the request deadline cancels the wait, and a started closure
always completes. Each write closure is one `BEGIN IMMEDIATE` transaction. Time is INTEGER
UTC unix seconds; timers are monotonic. `kohaku.db` is created 0600 before SQLite opens it.

**D8. Migrations and schema 1.** `migrations/NNNN_<change>.sql` (`NNNN` = resulting
`user_version`), embedded via `include_str!`, checked against the directory.

- Under the lock: downgrade guard → version 0 with a schema refused ("not a Kohaku
  database") → keycheck → pre-migration copy only when 1 ≤ version < newest (D31.6) → §3
  steps; a `foreign_key_check` failure names the migration.
- Every temporary file in `/data` and `/data/backups` is a `.tmp` made with `create_new`,
  mode 0600. The copy is `VACUUM INTO` such an empty file, flushed and renamed to
  `pre-migrate-v{old}-{unixtime}.db`; only then are older copies pruned to two.
- `meta`: one row (`CHECK (id = 1)`), keycheck, created_at; later settings become columns.
  `audit_log`: §4 columns, specs/audit-log rules as `CHECK`s (`GLOB` identifier shapes,
  label/id and target/id consistency), triggers aborting every `UPDATE` and a `DELETE` of
  any entry ≤ 365 days old; columns frozen, since restore writes into every schema.
- `outbox`: recipient user id or address (`CHECK` exactly one), subject, body, priority,
  token and placeholder flags, attempts, next attempt, queued, expiry, outcome. `user_id`
  has no FK until `users` exists; `report_id`/`project_id` arrive with their parents via
  `ALTER TABLE … ADD COLUMN … REFERENCES … ON DELETE CASCADE` (verified; D31.4).
- `outbox`, `audit_log` and every table whose ids appear in audit entries use `INTEGER PRIMARY
  KEY AUTOINCREMENT`, so no id is reused (outbox outcomes are written by row id).

**D9. Lock, backup and restore.** `serve` and `restore` hold `File::try_lock()` on
`kohaku.lock` (created 0600), an OS lock, so it also excludes `docker compose run`; other
commands run without tokio. `backup <file>` opens `kohaku.db` without creating it, checks the
keycheck (no downgrade guard), runs `VACUUM INTO` a `.tmp` beside the target, publishes with
`hard_link` plus unlink, flushing file and directory: `link()` fails on anything at the target,
a dangling symlink or racing entry included; `rename()` would replace it. `backup -` streams a
snapshot from `/data/backups`. `restore`: lock → copy to a `.tmp` in `/data` → flush →
`integrity_check` and version ≥ 1 → insert `instance.restore` into the candidate → flush →
remove live `-wal`/`-shm` → rename → flush `/data`: one rename, and a candidate lacking
`audit_log` fails.

**D10. Server and shutdown.** Bind `[::]:8080` dual-stack, falling back to `0.0.0.0:8080` only
on `EAFNOSUPPORT`, after config, lock, keycheck and migrations. hyper-util `auto::Builder` in
`http1_only()` with `TokioTimer` and `header_read_timeout(10 s)` (armed for every request head,
keep-alive included); hyper's default header limits (100 headers, ~408 KiB buffer) are kept.
The peer address goes into request extensions; accept errors are counted and followed by a
short pause. SIGTERM/SIGINT: stop accepting, `GracefulShutdown` drains ≤ 8 s, background tasks
stop at their next await (each step is one transaction), exit 0. Not `axum::serve` (§3): it has
no header read timeout.

**D11. Request pipeline.** The nesting is specs/http-security's rejection order: a multipart
GET with an empty bucket gets 429 and takes no token; a POST to a GET-only path gets the
origin check, then 405. Layer errors use the entry's format and never echo input.

```
hyper-util connection (peer address → extension)
└─ header layer (outermost): fixed headers; CSP and Cache-Control only if unset
   └─ dispatch: /healthz, tls-ask → internal router (any Host; GET/HEAD else 405;
      │           no client address, rate class, tracing or permits)
      ├─ Host not in map → 421      ├─ main → admin isolation → main router
      └─ project → project router (project id as request extension)
         Router::layer (after matching): TraceLayer (MatchedPath) → client address
         per entry: cache class → error body → deadline → origin check (non-GET/HEAD)
                    → rate class → multipart 415 → body cap → MethodRouter
```

**D12. Route table and security matrix.** `routing::table()` returns every route as data:
host kind (internal, main, project, both), pattern, example path, `MethodRouter`, and each
declaration of specs/http-security and specs/request-limits (access, cache class, own CSP,
body cap, deadline, multipart, header-less exemption, rate classes, error format). No
`Default`, so a new declaration fails to compile at every entry; each router's fallback is
an entry (`no-store`, `read`, 404). Only `routing::build` calls axum's registration methods
(D3); tests feed it synthetic tables to cover caps, deadlines, permits and exemptions
foundation does not ship. `tests/route_matrix.rs` walks the table and asserts on the wire
per entry, method and host kind (headers, cache, CSP, origin outcomes, 404/421, 413, 415,
429, no cookie), matching declaration enums without wildcards, with explicit allowlists for
exceptions. Why: a route cannot exist unlisted, so the matrix cannot miss one.

**D13. Headers and cache classes.** One tower middleware removes and re-inserts the fixed
headers (once each, unweakenable by handlers) and applies §7's set-if-absent rule with a
`no-store` fallback, so pre-routing responses get the full set; one function lists the
policy (D31.2). The dev CSP is a second constant; exact-header tests compile only without
`dev`. Classes, set per entry: `NoStore`, `NoCache`, `StaticAsset` (`immutable` only on a
200 GET/HEAD of a hashed asset, else `no-cache`).

**D14. Origin check, admin isolation, caps, deadlines.** The origin check is per entry, so
it covers every non-GET/HEAD request reaching a host router, fallback included, before the
body; duplicate `Origin` or `Sec-Fetch-Site` → 403. Admin isolation wraps the main router
already. Body cap: `RequestBodyLimitLayer` plus `DefaultBodyLimit` at the same value
(D31.3), one class for now (64 KiB, 15 s); `multipart/*` → 415 on any method of a
non-accepting entry. Deadline: `TimeoutLayer` (408) per entry; permits are RAII.

**D15. Host map, internal endpoints, URL builder.** `HostMap` (normalized host → `Main` |
`Project(id)`) sits in a `RwLock<Arc<_>>`; `projects` rebuilds by swapping the `Arc`. One
project router serves all project hosts, the id a typed request extension, not a path
parameter; tests add synthetic project entries. tls-ask hand-parses exactly one `domain`
(percent-decoded, normalized like Host) and answers 200 only for a project entry (D31.5).
`/healthz` answers from memory (binding after startup makes 200 mean "started"); a database
probe would be an unlimited public endpoint. No URL builder function takes a request.

**D16. Client address and limits.** `limits/client_ip.rs` implements §3 Client IP (warnings are
`AtomicBool`s). One `Mutex<HashMap>` holds every class and the per-source mail buckets; refill
takes an `Instant` from the caller, so tests drive time without tokio `test-util`. An IPv6
request needs a token in both its /64 and /48 bucket before taking either. `RateClass` has one
variant, `Read`. `limits/permits.rs` holds named `try_acquire` semaphores (public writes: 8).
The public mail budget checks and takes both its buckets in one step under the lock.

**D17. Mail outbox and SMTP.** A `Mailer` trait (`send` → failure class, reply code),
implemented over lettre, as D18's printer and as an in-memory fake; the message builder can
only produce one `text/plain; charset=utf-8` part from `KOHAKU_SMTP_FROM` to one bare `To`.
`TlsParameters` are built offline at startup from the `SmtpRoots` passed in (`serve`: only
`compiled()`, lettre's `CertificateStore::WebpkiRoots`), `Tls::Wrapper` for `implicit`,
`Tls::Required` for `starttls`, EHLO = main host; lettre's error text is never logged. A mail
kind is a `const` descriptor from the sending change (priority, token flag, lifetime, and for
token kinds a give-up action invalidating the token in the same transaction); `enqueue` runs
in the caller's write transaction. One worker, woken by a `Notify` or the next due time, picks
by priority, next attempt, queue time, runs each attempt as its own task under a 60 s timeout
(a panic or stall is one failure), one connection at a time, deletes placeholder rows unsent
and writes results by row id, so a row deleted meanwhile stays deleted. All unsent-row
removals share one give-up function.

**D18. Development mail and SMTP tests.** No mail server anywhere in development (owner
decision). In a `dev` build `serve` uses `mail::dev::PrintMailer`: it writes the envelope, the
headers of lettre's formatted message and the row's body text between `--- kohaku dev mail
<row id> ---` and `--- end ---` to stdout, then reports acceptance, so the worker runs
unchanged. The SMTP adapter compiles in every build, so both test builds cover it:
`tests/smtp_tls.rs` runs a local rustls peer on 127.0.0.1 with committed fixture certificates
(a test CA, a `localhost` leaf, an expired leaf, another-host leaf, a self-signed leaf; made
once with openssl by `tests/fixtures/smtp/generate.sh`, far-future validity, output committed,
so no dev shell needs openssl) and passes the adapter `SmtpRoots::for_tests(<test CA>)`. The
compiled roots first meet a real provider in the first change that sends mail, as a manual
check there. Rejected: Mailpit or any local SMTP catcher (a container plus a dev CA, since
plaintext is forbidden); a plaintext dev mode.

**D19. Audit.** `audit(tx, actor, action, target)` writes in the caller's transaction with
typed `Actor` (`Cli`; users with `admin-auth`), `Action` enum (`InstanceRestore` →
`instance.restore`) and `Target`; no parameter takes a string. D8's constraints back it up.

**D20. Jobs.** One scheduler task: limiter sweep and counter flush every 60 s; retention
after startup and hourly, then `wal_checkpoint(TRUNCATE)`. Retention is an ordered list of
steps, one transaction each, a failure logged without row content and retried next run:
outbox rows (unsent ones through give-up), audit entries > 365 days, `.tmp` files > 24 h
directly in `/data` and `/data/backups` (`symlink_metadata`, no link following or descent).

**D21. Logging.** `fmt` without ANSI at a fixed level (INFO, DEBUG in `dev`). Our own
`make_span_with` records only `MatchedPath` and status. Handler 5xx log at ERROR; rejections
mark their response and only increment per-reason counters (421, origin 403, admin-isolation
403, 408, 413, 415, 429 per class, 503 per bound, accept errors), flushed as one INFO line per
reason per minute. Dependency errors become fixed kinds at module boundaries.
`tests/logging.rs` sends marker secrets, addresses, paths, queries, cookies and bodies to every
entry; none may reach TRACE output.

**D22. Assets and templates.** `include_bytes!` assets named with 16 hex digits of their
SHA-256, computed at startup (a `build.rs` would add our crate to the build-script surface);
the static entry answers only from that registry. `base.html` links the source repository
(AGPL §13); a test fails on `|safe` or disabled escaping (markdown will use `HtmlSafe`). The
landing page is static text without JavaScript or forms.

**D23. `dev` feature and loop.** `just dev` creates once the gitignored `.env.dev` holding a
random `KOHAKU_SECRET` (the dev database's keycheck needs the same one every start), sets
`KOHAKU_BASE_URL=http://localhost:8080`, `KOHAKU_TRUSTED_PROXIES=none` and
`KOHAKU_SMTP_FROM=kohaku@localhost`, and runs `watchexec -r` over `cargo run --features dev --
serve`. `dev` changes exactly: the CSP; CSS/JS read from `static/` per request (by registry
name, never a joined request path); `/static/dev-reload.js` and `/dev/boot-id` as table entries
on both routers; `http://localhost` base URLs accepted; mail printed, SMTP connection settings
refused (D18); `./data`; DEBUG. All of it sits in `dev.rs`, `mail/dev.rs` or cfg'd constants.

**D24. Nix flake.** nixpkgs pinned to `openspec` 1.13.1, rust-overlay following it, x86_64
and aarch64 Linux, no flake-utils. `packages.default` as §11, with a `lib.fileset` source
(each directory joins in the task creating it, plus `.env.example`) and tests on, so `nix
build` runs them; host platform only. The devShell adds `file` (D31.9). Outside Nix,
`docs/dependencies.md` names `cc` and `file`.

**D25. Dockerfile.** As §13: one COPY-only stage from `dist/${TARGETARCH}/kohaku` (with
`--chmod=0755`: CI artifacts lose the exec bit) and `docker/data/.keep`, no `USER` (base is
nonroot) or `HEALTHCHECK` (Compose owns it); `.dockerignore` admits only those and
`Cargo.lock`; CI and release build from it.

**D26. Compose.** As §14, plus: SMTP settings and both secrets interpolated from `.env`, the
secrets as `${…:?…}` with a how-to message; image pinned to the major tag; `.env.example`
holds `KOHAKU_DOMAIN`, the empty secrets and the SMTP settings, with D5's refused placeholders
and real defaults (587, `starttls`).

**D27. CI (`ci.yml`).** §13 rules on `push` and `pull_request`; jobs on `ubuntu-24.04`;
`permissions: {}` with `contents: read` per job; `persist-credentials: false`; no caches;
toolchain via rustup, cargo-deny and cargo-zigbuild via `cargo install --locked …@=X.Y.Z`, zig
as a SHA-256-checked tarball. Jobs: `check` (fmt, clippy, both test builds), `deny`, `build`
(x86_64 and aarch64 musl, uploads binary and hash; D31.8), `smoke` (needs `build`, D29).

**D28. Release (`release.yml`).** Names as §13 and `docs/releasing.md`. `build` checks the tag
is exactly `cargo metadata`'s `X.Y.Z`, then advisories, `zigbuild`; `arm-check` on
`ubuntu-24.04-arm` verifies the hash, then compares `kohaku --version` with the tag; `publish`
builds the index once into a local OCI archive and fails unless its SBOM lists `pkg:cargo/`
packages (D31.7), then pushes by digest, signs, tags `X.Y.Z` last. Later changes leave it
alone.

**D29. Smoke test.** `ci/smoke.sh` (CI, `just smoke`) checks Docker Engine ≥ 27; tags the
release-Dockerfile image as `docker compose config` names it, so the shipped `compose.yaml`
runs as is with `--pull never`; writes a valid `.env` (0600, secrets included) with
`KOHAKU_CADDY_CI` (`local_certs` and `skip_install_trust` on separate lines); waits for healthy
on a fresh volume; expects via Caddy's root certificate the landing page with CSP and HSTS,
tls-ask 404 and no handshake for an unknown name; runs `backup -` via `exec` and `restore -`
via `run` (catches an ENTRYPOINT), then requests `/` again; `down -v` always.

**D30. Test harness.** `oneshot` on the assembled service, or a real listener on 127.0.0.1:0
with a raw-socket client where hyper matters (header timeout, peer address, `healthcheck`,
shutdown). Configuration from maps, limiter and scheduler time injected (no tokio `test-util`),
the fake `Mailer`, temporary data directories; only `cli.rs` knows the fixed `/data` path.

**D31. Deviations from the design.** All other gaps are sequencing (Non-Goals).

1. **No optional TOML file** (§3 Config), owner decision: Compose uses the environment; a
   second source needs precedence rules and toml. `/kohaku.toml` leaves `.gitignore`.
2. **Crate list** (§3): toml dropped; tower, hyper-util, base64, idna_adapter direct;
   headers from own middleware, not tower-http (D13).
3. **Body caps twice** (§6): `RequestBodyLimitLayer` refuses oversize bodies unread.
4. **Outbox references later** (§4): `user_id` without FK; `users`' change adds the account
   recipient and either rebuilds `outbox` with the FK or deletes rows in code (§9);
   `report_id`, `project_id` and the §4 cascades come with `projects`, `report-submission`
   and `moderation`.
5. **tls-ask 404 for the main domain** (§6): it has its own Caddy site block.
6. **No pre-migration copy for a new database** (§3 rule 2): nothing to preserve.
7. **SBOM checked on every release** (§13 checks only the first).
8. **CI builds both musl targets** (§13): catches aarch64 breakage before a tag.
9. **Tools** (§11): `file` in the devShell; non-Nix setup names a system `cc`.
10. **Releases only from the newest line** (§13): no backports, so moving `X.Y`, `X`,
    `latest` never sends `docker compose pull` to a line the downgrade guard refuses.

## Risks / Trade-offs

- [Hand-written parsers (arguments, base URL, DNS names, CIDR, XFF, tls-ask)] → Small,
  strict, covered by the specs' adversarial cases.
- [Crash after SMTP acceptance resends] → At-least-once; duplicates carry the same token.
- [Pre-migration copy of a large database is slow, uses disk] → Two kept; steps logged.
- [`net.ipv6.bindv6only=1` yields an IPv6-only listener] → The healthcheck fails at once.
- [No cap on open connections] → Header timeout, deadlines, proxy-only exposure.
- [Fixed `/data` outside containers] → Bind mount or `BindPaths=`; configurable later.
- [A cached page names an old asset hash] → Pages are `no-cache`; old names 404.
- [Allowlists block Dependabot] → Intended: a human accepts each license or build script.
- [Test CA key committed under `tests/`] → Trusted only through `SmtpRoots::for_tests`, which
  the source scan keeps out of `src/`.
- [Secrets in the environment are visible to `docker inspect` and the process's `environ`] →
  Owner decision for one settings file; Docker access is root-equivalent, `.env` is 0600, the
  kohaku uid already owns the data, and nothing logs the environment.

## Migration Plan

No instance exists; the first `serve` creates schema 1 and its keycheck in one transaction.
Build order: skeleton, flake, policy files and CI; configuration and keys; storage; pipeline
and route matrix; limits; mail; audit, jobs, logging; assets; dev loop; image, Compose and
smoke test; release last. The release workflow stays inert until the owner completes
`docs/releasing.md`. Rollback: before a release, revert; after, as §10.

## Open Questions

- Tag `0.1.0` from foundation to exercise publishing (signing, immutable tags, SBOM check)?
  It moves `latest` to a featureless instance; an owner call at tag time.
- Exact pins (toolchain patch, zig, cargo-zigbuild, cargo-deny, just, watchexec, Caddy,
  distroless) are set at apply time in `docs/dependencies.md`.
- A footer source-link setting for modified builds (AGPL §13)? Addable later.
