# Tasks

## 1. Toolchain, repository skeleton and CI

- [ ] 1.1 Add `rust-toolchain.toml` pinning stable `1.NN.P` (D3; distribution: Pinned toolchain and
  development environments); verify `rustup show active-toolchain` names it.
- [ ] 1.2 Create `docs/dependencies.md` with the pin and the non-Nix prerequisites (D24, D31.9;
  Debian/Ubuntu: `build-essential`, `file`); verify `rustc --version` shows the pin and `cc` and
  `file` run.
- [ ] 1.3 Create the `kohaku` package per D3 and D4 (`version = "0.1.0"`, AGPL-3.0-or-later,
  `publish = false`, committed `Cargo.lock`); verify `cargo build --locked` passes.
- [ ] 1.4 Add the `compile_error!` for `dev` in release builds (distribution: Development feature
  excluded from release builds); verify `cargo build --release --features dev` fails with it and
  `cargo build --features dev` passes.
- [ ] 1.5 Add `cli.rs`, a hand-written `args_os()` match (D2), with `--version`, `--help` and usage
  errors (operations: Command-line commands, Usage errors); verify grammar unit tests and
  `cargo run -- --version` printing `kohaku 0.1.0`.
- [ ] 1.6 Add `flake.nix`, `flake.lock` and `packages.default` (D24; later tasks add what they
  create to its fileset); verify, with Nix or else the `nixos/nix` Docker image, that
  `nix flake check` and `nix build` pass.
- [ ] 1.7 Add the devShell and `.envrc` (D24; distribution: Pinned toolchain and development
  environments); verify in `nix develop` that `rustc --version` shows the pin, `openspec --version`
  shows `1.13.1`, and `node` and `npm` are absent.
- [ ] 1.8 Record in `docs/dependencies.md` the devShell's just, watchexec-cli and cargo-deny
  versions (for `cargo install --locked` outside Nix) and the cargo-zigbuild and zig pins; verify
  they match `--version` in `nix develop`.
- [ ] 1.9 Add `deny.toml` (D3; distribution: Dependency policy enforced in CI); every later "adds
  `<crate>`" extends it and `docs/dependencies.md`; verify `cargo deny check` passes, and fails with
  `openssl-sys`, a git source or an unlisted build script.
- [ ] 1.10 Add `clippy.toml` with the `std::env` bans (later crates bring theirs) and
  `tests/source_scan.rs` (D3), adding `tests/` to the fileset; verify clippy `-D warnings` passes,
  the scan fails on an exemption planted outside `config.rs`, and `nix build` runs the scan.
- [ ] 1.11 Add `ci.yml` with the `check` and `deny` jobs (D27; distribution: Continuous integration
  without secrets); verify actionlint (in Docker) and a `grep` for forbidden triggers, cache
  actions, `openspec`, `npm` or `node` steps and unpinned `uses:`.
- [ ] 1.12 Add the `build` job for both musl targets (D27, D31.8); verify actionlint and that `file`
  reports a local x86_64 musl `cargo zigbuild` binary as statically linked.
- [ ] 1.13 Add `.github/dependabot.yml` (distribution: Dependency policy enforced in CI); verify it
  groups monthly `github-actions`, `docker` and `cargo` updates.

## 2. Configuration

- [ ] 2.1 Add the shared DNS-name validator and the `KOHAKU_BASE_URL` parser (D5; configuration:
  Base URL format); verify unit tests for "Base URL values", `http://localhost:8080` passing only
  under `dev`.
- [ ] 2.2 Add the `KOHAKU_TRUSTED_PROXIES` parser and the CIDR-matching trusted set (D2, D16;
  configuration: Trusted proxy list format); verify unit tests for "Accepted and rejected lists".
- [ ] 2.3 Add the SMTP setting parsers (D5; configuration: SMTP settings); verify unit tests for
  "Malformed SMTP values rejected" and the values of "Unreachable SMTP server does not block
  startup".
- [ ] 2.4 Add the `KOHAKU_PUBLIC_MAIL_PER_HOUR` parser (configuration: Settings and defaults);
  verify unit tests for "Public mail budget values".
- [ ] 2.5 Add the temporary-directory helper `src/test_support.rs` (D30), `#[cfg(test)]` and shared
  with `tests/support/`; verify unit tests: unique names, no existing entry reused, removal on drop.
- [ ] 2.6 Add the `KOHAKU_SMTP_PASSWORD` type without `Debug` or `Display` and error reporting
  that names variables only (D5; configuration: Startup validation and error reporting); verify
  unit tests that no valid, malformed or empty secret value appears in any error.
- [ ] 2.7 Add instance-secret decoding into `keys::InstanceSecret`, without `Debug` or `Display`
  (D6; configuration: Instance secret); adds `base64`; verify unit tests for "Secret encoding".
- [ ] 2.8 Add `Config::load(command, lookup)` and a test-config helper (D5; configuration:
  Configuration sources, Startup validation and error reporting); verify "Every problem is reported
  in one run" and "Commands need only their own settings".
- [ ] 2.9 Make a `dev` build read only `KOHAKU_SMTP_FROM` of the SMTP settings and refuse the other
  five when set (D5, D18; configuration: Settings and defaults); verify `--all-features` unit tests
  for "Development build takes no SMTP server" and that 2.3's tests compile only without `dev`.
- [ ] 2.10 Drop `/kohaku.toml` from `.gitignore` (D31.1); verify `git check-ignore kohaku.toml`
  matches nothing.
- [ ] 2.11 Add `.env.example` (D26; distribution: Secrets and settings in the environment file)
  and refuse its placeholders; verify "Unchanged example configuration" in an `include_str!` test,
  and that `git status` ignores `.env` and `.env.dev` but tracks `.env.example`.
- [ ] 2.12 Write the README configuration section; verify it gives each setting of configuration:
  Settings and defaults as required or optional with format and default, and
  `KOHAKU_TRUSTED_PROXIES` as exact addresses normally, CIDR blocks as a last resort.

## 3. Keys

- [ ] 3.1 Add MAC framing, constant-time verification and `Purpose` (only `Keycheck`) in `keys.rs`
  (D6; configuration: Instance secret); adds `hmac`, `sha2`; verify "MAC inputs cannot collide",
  unique `kohaku/` labels and a keycheck stable per secret.
- [ ] 3.2 Add `PerBootKey::generate()` (D6; configuration: Per-boot keys); adds `getrandom`; verify
  two keys differ and `compile_fail` doc tests for `{:?}` and `{}` on `PerBootKey` and
  `InstanceSecret`.

## 4. Storage and migrations

- [ ] 4.1 Add the `db/` connections and `time::now_unix()` (D7; data-storage: Database file and
  connection settings); adds `rusqlite`; verify each connection's pragmas, mode 0600, and failing
  reader writes, STRICT and foreign-key violations.
- [ ] 4.2 Add `db.write` and `db.read` over semaphore permits and `spawn_blocking` (D7); adds
  `tokio`; verify "Concurrent reads stay within the connection bound" and a write waiting out a 2 s
  write transaction.
- [ ] 4.3 Add the check-and-consume write helpers (D7; data-storage: Atomic limit enforcement);
  verify concurrency tests for "Parallel attempts".
- [ ] 4.4 Add the migration runner over a migration-list parameter (D8; data-storage: Schema version
  and migrations at startup); verify synthetic-list tests for "New, older and current databases",
  "Failed or interrupted migration" and a failing SQL statement in k leaving k-1.
- [ ] 4.5 Add the downgrade guard and the refusal of version 0 with a schema (D8; data-storage:
  Downgrade guard); verify a version-4 database under newest migration 3 is refused naming both and
  unchanged.
- [ ] 4.6 Add `migrations/0001_foundation.sql` with `meta` (D8), embedded and checked against the
  directory; verify that check and `nix build`.
- [ ] 4.7 Add the design §12 migration test with per-table sample rows; verify data-storage "Table
  rebuilds keep child rows" and a failure for a table lacking a sample row.
- [ ] 4.8 Wire the keycheck into the runner (D6, D8; configuration: Database keycheck); verify
  runner tests for "Wrong secret refused" and a keycheck-less database refused by backup and
  completed by serve.
- [ ] 4.9 Add the pre-migration copy (D8, D31.6; data-storage: Pre-migration copy); verify its two
  scenarios, a missing `/data/backups` created 0700, no copy for new or current databases, and none
  after a secret mismatch.
- [ ] 4.10 Add the instance lock (D9; data-storage: Instance lock); verify `kohaku.lock` mode 0600,
  a second lock refused with no file changed, and a SIGKILLed holder's lock retaken without deleting
  `kohaku.lock`.
- [ ] 4.11 Add `backup <file>` (D9; data-storage: Backup to a file); verify its two scenarios,
  configuration "Backup refused with the wrong secret", a newer-schema database backed up at its
  version, and a missing database failing with no file created.
- [ ] 4.12 Add `backup -` into any `Write` (D9; data-storage: Backup to standard output); verify
  output that is only a checked SQLite file, the `.tmp` 0600 while streaming and a missing
  `/data/backups` created 0700, no `.tmp` left, an error when the writer fails after 4 KiB, and no
  byte written on a failure before the snapshot (e.g. wrong secret).
- [ ] 4.13 Add the backup listing (D9; operations: Listing backups); verify its scenario.

## 5. Host routing and server

- [ ] 5.1 Add host normalization (D15; host-routing: Host normalization); verify unit tests for each
  `Host` value of its scenarios.
- [ ] 5.2 Add `HostMap` in a `RwLock<Arc<_>>` (D15; host-routing: In-memory host map); verify unit
  tests for "Attacker uses lookalike or random hosts" and a base URL port not affecting the match.
- [ ] 5.3 Add `routing::table()`, the declaration types in their final modules and `routing::build`,
  sole caller of axum's registration methods (D3, D12); adds `axum`; verify clippy rejects a call
  elsewhere and a synthetic table builds.
- [ ] 5.4 Add the dispatcher (D11, D15; host-routing: Unknown hosts get 421, Main-host routes,
  Project-host routes); adds `tower`; verify `tests/hosts.rs` with a synthetic project host for the
  421 and 404 cases of these, Host normalization and In-memory host map.
- [ ] 5.5 Add the per-reason rejection counters (D21), which every later rejection increments,
  starting with unknown host; verify 1000 unknown hosts add 1000.
- [ ] 5.6 Add `/healthz` (D15; operations: Health endpoint; host-routing: Internal endpoints are
  matched before host routing); adds `serde`, `serde_json`; verify "Attacker input is neither
  reflected nor answered with internals" and "Attacker probes around the internal endpoints".
- [ ] 5.7 Add tls-ask (D15, D31.5; host-routing: TLS ask answers only for configured project hosts);
  verify its three scenarios with a synthetic project host.
- [ ] 5.8 Add the URL builder (D15; host-routing: Absolute URLs come only from configured hosts);
  verify unit tests for its two scenarios.
- [ ] 5.9 Add `http/server.rs` and a raw-socket test client (D10; http-security: Request header read
  timeout); adds `hyper-util`; verify its scenario, also on keep-alive, and no `h2` in `cargo tree`.
- [ ] 5.10 Add `tests/route_matrix.rs`, which later tasks extend (D12; http-security: Per-route
  security declarations; host-routing: Cookies only from main-host handlers with the __Host-
  prefix); verify it passes, including host-routing "No cookies in this change, even for a forged
  one", and a new declaration variant fails to compile.

## 6. HTTP security layer

- [ ] 6.1 Add the header layer (D13; http-security: Security headers on every response, Exact
  Content Security Policy); verify `tests/headers.rs` (non-`dev`) for "Every response carries the
  header set" so far, a handler's value replaced, and matrix rows for "Route table matches its
  declarations", failing on an own CSP or Cache-Control outside the allowlist.
- [ ] 6.2 Add the cache-class layer (D13; http-security: Cache-Control classes); verify its scenario
  for the routes so far and a matrix row per entry's class.
- [ ] 6.3 Add the Fetch-Metadata/Origin layer (D14; http-security: Fetch-Metadata and Origin check
  on state-changing requests); verify `tests/fetch.rs` for its four scenarios (`POST /` passing to a
  404 until 11.4) with synthetic project-host and exempt entries, and matrix rows: `Origin: null`,
  `Sec-Fetch-Site: cross-site` and header-less get 403 on every non-exempt entry.
- [ ] 6.4 Add admin resource isolation (D14; http-security: Resource isolation for admin GET
  requests); verify its scenario, each 403 with the full header set.
- [ ] 6.5 Add the body cap (D14, D31.3; http-security: Default request body cap of 64 KiB); adds
  `tower-http`; verify its scenario on synthetic body-reading entries and a matrix 413 row.
- [ ] 6.6 Add the multipart rule (D14; http-security: Multipart bodies rejected unless explicitly
  accepted); verify its scenario and a matrix 415 row.
- [ ] 6.7 Add the request deadline (D14; http-security: Request deadline); verify a trickled body
  and a slow handler get 408 at 15 s with the full header set.
- [ ] 6.8 Add the log subscriber (D21); adds `tracing` and `tracing-subscriber`; verify
  `cargo tree -e features` shows no `env-filter`.
- [ ] 6.9 Add request tracing, banning `TraceLayer::new_for_http` (D21; http-security: Request
  tracing records only route pattern and status); verify a TRACE capture test for its scenario (a
  synthetic static route until 11.1) and a path-parameter route's pattern.

## 7. Request limits

- [ ] 7.1 Add client-address resolution (D16; request-limits: Trusted proxies and the forwarded-for
  header, Client address resolution scope); verify its four scenarios and configuration "Attacker
  sends a forwarded-for header with no trusted proxy".
- [ ] 7.2 Add the warnings (D16; request-limits: One-time client-address warnings); verify
  real-listener tests for its two scenarios and host-routing "Probes with internal Host values".
- [ ] 7.3 Add the token buckets with injected time and the `RateClass::Read` budgets (D16;
  request-limits: Rate-limit keys by network prefix); verify unit tests for its three scenarios and
  a /48 regaining one token per 25 ms.
- [ ] 7.4 Add the bucket cap, overflow buckets and sweep (D16; request-limits: Bounded limiter
  memory); verify unit tests for its two scenarios.
- [ ] 7.5 Apply `read` per entry (D12, D16; request-limits: Read rate-limit class, Rate-limited
  requests are rejected before any work); verify their scenarios, both internal-endpoint flood
  scenarios (`/` a 404 until 11.4), admin-isolation 403s taking no read token, and a matrix 429 row.
- [ ] 7.6 Add the public mail budget, uncalled for now (D16; request-limits: Public mail budget);
  verify unit tests for its three scenarios.
- [ ] 7.7 Add the public-write permits (D16; request-limits: Bounded public write concurrency);
  verify "Attacker floods public writes", http-security "Permit is released at the deadline" and
  operations "Attacker floods the health endpoint".

## 8. Mail outbox

- [ ] 8.1 Add the subject sanitizer and user-text transform (D17; mail-outbox: User-supplied text in
  mail is single-line, capped and marked as quoted); verify unit tests for its two scenarios and the
  subject cut.
- [ ] 8.2 Add the recipient validator (mail-outbox: Outbox rows name one validated recipient);
  verify unit tests for "Header injection through the recipient address is refused".
- [ ] 8.3 Add the message builder (D17; mail-outbox: Mail is a single plain-text part); adds
  `lettre` and `idna_adapter`; verify "CR LF in a subject cannot add headers" and no ICU4X, openssl,
  native-tls, aws-lc or rayon in musl `cargo tree`.
- [ ] 8.4 Add the `outbox` table and its sample row (D8, D31.4); verify the database refuses a row
  with both or neither recipient, and deleting the newest row never lets its id be reused.
- [ ] 8.5 Add the mail-kind descriptors and `enqueue`, which wakes the worker (D17; mail-outbox:
  Mail is sent only through the persistent outbox); verify "Rolled-back change queues no mail", no
  row for a bad address, and a new row due at once.
- [ ] 8.6 Add the retry schedule and deadline as pure functions (mail-outbox: Failed attempts are
  retried on a fixed schedule); verify unit tests for "Full retry timeline".
- [ ] 8.7 Add the `Mailer` trait, its fake and the worker (D17); verify fake-mailer tests for
  "Public mail flood does not delay security mail", "Acceptance defines sent" and row A of "Restart
  keeps the schedule".
- [ ] 8.8 Add delivery outcomes and the shared give-up function (D17; mail-outbox: Token-bearing
  rows are deleted when sent or given up); verify its two scenarios, "Placeholder row is never sent"
  and row B of "Restart keeps the schedule".
- [ ] 8.9 Add the SMTP adapter over `SmtpRoots` (D17; mail-outbox: SMTP delivery requires verified
  TLS); verify with a scripted plaintext peer configuration "Attacker strips STARTTLS" and a broken
  handshake, and construction with nothing listening.
- [ ] 8.10 Add `tests/smtp_tls.rs`, the fixture certificates with `generate.sh`, and `rustls` as the
  only dev-dependency (D1, D18); verify "Verified server" for `implicit` and `starttls`, the
  received message being one `text/plain; charset=utf-8` part with the configured `From` and one
  `To`; the self-signed, expired and other-host leaves and an unpassed CA failing before `AUTH`;
  and the source scan failing on a planted `SmtpRoots::for_tests` in `src/`.
- [ ] 8.11 Reduce lettre errors to failure class and reply code (D21; mail-outbox: SMTP credentials
  and mail data stay out of logs and errors); verify unit tests for its scenario.
- [ ] 8.12 Run the worker on the real adapter; verify "Stalled SMTP server does not hold requests"
  with a synthetic enqueueing route, a dead address and a silent listener.

## 9. Audit log, restore and retention

- [ ] 9.1 Add the `audit_log` table, `CHECK`s, triggers and sample row (D8; audit-log: Entry fields,
  Append-only); verify "Entry read back", "Malformed, inconsistent or attacker-supplied values
  refused" and "Attacker tries to rewrite or erase a recent entry" as SQL tests (an actor change
  refused; a 364-day `DELETE` refused, a 366-day one allowed).
- [ ] 9.2 Add `audit(tx, actor, action, target)` with typed `Actor`, `Action` and `Target` (D19;
  audit-log: Ids only, Recorded together with the change); verify "Entry and change are atomic".
- [ ] 9.3 Add the schema test mapping each `Target` and user `Actor` to an `AUTOINCREMENT` table
  (audit-log: Entries outlive actors and targets); verify it and both scenarios on synthetic tables.
- [ ] 9.4 Add the request-flood audit test (audit-log: Audited actions); verify "Anonymous request
  flood".
- [ ] 9.5 Add restore (D9; data-storage: Restore; audit-log: Restore is audited); verify their
  scenarios, "Restore entry carries no file name or content", "Restore skips the keycheck", an older
  backup migrated at the next start, a pre-migration copy opened by the older runner, and a
  version-0 source and one lacking `audit_log` refused with the live files unchanged.
- [ ] 9.6 Add the scheduler (D20, time injected per D30); verify with injected time retention after
  startup and every 60 min, the sweep every 60 s, and the stop on shutdown.
- [ ] 9.7 Add the retention steps (D20; data-storage: Retention job); verify its three scenarios and
  audit-log "Retention purges only old entries".
- [ ] 9.8 End each retention run with `wal_checkpoint(TRUNCATE)` (data-storage: Secure deletion and
  WAL checkpoints); verify its scenario and a blocked checkpoint failing no request.
- [ ] 9.9 Verify request-limits "Only public writes need permits, and failures return them" and
  audit-log "Read-only commands and background work" as tests.

## 10. Operations and CLI

- [ ] 10.1 Add the `backup` and `restore` CLI arms and their `--help`, without tokio (D2, D9;
  operations: Command-line commands, Usage errors); verify binary tests for their part of "Malformed
  invocations" and "Version and help need no configuration".
- [ ] 10.2 Map refusals to exit 1 and keep stdout data-only (operations: Backup and restore through
  standard streams); verify its two scenarios, "A refusal is a failure, not a usage error", and
  empty stdout from successful `backup <file>`, `restore <file>` and `restore -`.
- [ ] 10.3 Add the library `serve` entry point, its mailer, listener and shutdown injected, logging
  the effective `KOHAKU_PUBLIC_MAIL_PER_HOUR` (D5, D10, D30; operations: Server start); verify "No
  request is served before startup has finished", "A configuration file is ignored", that line, and
  an invalid setting leaving the data directory empty.
- [ ] 10.4 Verify as `serve` tests configuration "Wrong secret refused" and "Unreachable SMTP server
  does not block startup", data-storage "Concurrent commands and a crashed holder", and no audit
  entry from a migrating start.
- [ ] 10.5 Add the `serve` CLI arm, the runtime per §3 and the dual-stack bind (D10); verify its
  part of "Malformed invocations", `kohaku SERVE`, "Secret-bearing flags are refused" and
  "Reachable on IPv4 and IPv6 loopback" on port 0.
- [ ] 10.6 Add `kohaku healthcheck` (D2; operations: Healthcheck command); verify its two scenarios
  against local listeners, `--help` naming every invocation, `kohaku healthcheck extra` exiting 2,
  and an unchanged audit entry count after a healthcheck.
- [ ] 10.7 Add graceful shutdown (D10; operations: Graceful shutdown); verify its three scenarios
  through the library's shutdown trigger.
- [ ] 10.8 Flush the rejection counters every 60 s (D20, D21; operations: Rejected requests are
  logged as per-minute counters); verify its two scenarios.
- [ ] 10.9 Log to stdout in `serve` and to stderr elsewhere (operations: Command-line commands);
  verify binary tests: `serve` logs on stdout, a failing `backup` only on stderr.
- [ ] 10.10 Add `tests/logging.rs` (D21; operations: Log content privacy); verify its scenario on
  every table entry.
- [ ] 10.11 Add the leak test (configuration: Per-boot keys; operations: Secrets are never arguments
  or output; request-limits: Limiter state lives in memory only); verify their scenarios,
  "Delivered token mail leaves no copy", and no SMTP password bytes in the data directory or a
  backup.

## 11. Landing page, static assets and development loop

- [ ] 11.1 Add the asset registry and the static entry on both routers (D22; host-routing: Hashed
  static assets on both routers); verify its two scenarios, http-security "Each response gets its
  class" for the stylesheet (immutable 200; a changed hash 404 `no-cache`, no `max-age`), the
  stylesheet despite an empty read bucket and with `?email=…&token=…` logging no query, and the
  matrix and logging tests covering the static entry.
- [ ] 11.2 Add askama, `templates/base.html` and a test failing on `|safe` or disabled escaping
  (D22); verify that test, also against a planted `|safe`, and `nix build`.
- [ ] 11.3 Add `templates/error.html` and the error-body layer (D11, D22); verify "Attacker probes
  for the admin area", equal 421 bodies without the host, and no body echoing request markers.
- [ ] 11.4 Add the landing page (D22; host-routing: Main-host routes); verify its three scenarios,
  "Mixed case and a port are normalized", CSP-clean landing and error HTML (http-security: Exact
  Content Security Policy) whose only resources are hashed `/static/` paths and whose only absolute
  link is the footer's, and the `/` cases of 6.1 to 6.3, 6.5, 6.6 and 7.5.
- [ ] 11.5 Add `dev.rs` (D23; distribution: Development live reload): dev CSP, assets from
  `static/`, `./data`, DEBUG; verify `--all-features` tests that only `connect-src 'self'` differs
  from release and an edited stylesheet is served unbuilt.
- [ ] 11.6 Add `/dev/boot-id` and `/static/dev-reload.js`, loaded by `base.html` in `dev` only
  (D23); verify `--all-features` tests that two `/dev/boot-id` requests give one id and both routes
  answer on the main and a synthetic project host (the script as `text/javascript`), and
  distribution "Attacker probes a release build for development routes".
- [ ] 11.7 Add `mail/dev.rs`'s `PrintMailer`, used by `serve` in a `dev` build (D18; mail-outbox:
  Development builds print mail instead of sending it); verify an `--all-features` test for
  "Developer reads a one-time code in the terminal" and the source scan failing on a printer
  outside `cfg(feature = "dev")`.
- [ ] 11.8 Add `just dev` (D23); verify "Page reloads after an edit" in a real browser (by hand, or
  in Docker such as headless Chromium with `--network host`), recording results in the PR, that the
  first run creates `.env.dev` and later runs reuse it, and "Same toolchain with and without Nix".

## 12. Container image, Compose and smoke test

- [ ] 12.1 Add the `Dockerfile`, `.dockerignore` and `docker/data/.keep`, `dist/` ignored (D25;
  distribution: Container image contract); verify with local zigbuilt binaries (`file`: both static)
  its amd64 and arm64 builds, inspect showing user 65532, no entrypoint and the `CMD`, a working
  `kohaku --version`, a new volume at `/data` being 65532:65532 mode 0700, the layers and no shell.
- [ ] 12.2 Add `compose.yaml`'s `kohaku` service and `proxy` network (D26; distribution: Hardened
  Kohaku service in Compose, Private dual-stack proxy network); verify `docker compose config`: no
  kohaku port, the `KOHAKU_DOMAIN` error, "Environment cannot widen the trust list".
- [ ] 12.3 Add the `caddy` service and `Caddyfile` (D26; distribution: Hardened Caddy reverse
  proxy); verify `docker compose config` publishes only 80 and 443 and follows a changed
  `KOHAKU_DOMAIN` in both services, and `caddy adapt` without `KOHAKU_CADDY_CI` has no local issuer.
- [ ] 12.4 Add `ci/smoke.sh`, `just smoke` (zigbuilding x86_64 into `dist/amd64/kohaku` first) and
  the CI `smoke` job (D27, D29; distribution: Compose smoke test in CI); verify `just smoke` passes,
  also without a local Caddy image, "ENTRYPOINT regression fails CI" and actionlint.
- [ ] 12.5 Verify in a `just smoke` run, with `docker inspect`, "Compromised process is confined",
  "Unedited template never runs", kohaku at 640 MiB as uid 65532, and caddy with only
  `NET_BIND_SERVICE`, a read-only root and 256 MiB; verify "Secrets stay out of the repository and
  the shipped files" with `git status` and `grep`.

## 13. Release workflow and documentation

- [ ] 13.1 Add `release.yml`'s `build` and `arm-check` jobs (D28; distribution: Release build from a
  matching version tag); verify actionlint, "Tag not of the form X.Y.Z" with the tag check run
  locally, which accepts `1.2.3` against version `1.2.3`, and no cargo feature enabled.
- [ ] 13.2 Add the `publish` job (D28, D31.7; distribution: Owner-approved publish, Signed
  multi-arch image); verify actionlint, 1.11's greps, one `environment: release`, a local OCI index
  SBOM listing `pkg:cargo/`, names as in `docs/releasing.md`.
- [ ] 13.3 Complete `docs/dependencies.md` (D1, D24): per direct crate a one-line justification,
  declared and resolved features; the bundled SQLite version; the `idna_adapter` `cargo tree`
  comparison; every pin, with zig's SHA-256, the distroless digest and cosign; verify an entry per
  `Cargo.toml` dependency, 8.3's `cargo tree` check, and the recorded just, watchexec-cli and
  cargo-deny versions still equal `nix develop`'s.
- [ ] 13.4 Add README operations notes (data-storage: Secure deletion and WAL checkpoints): backups
  keep deleted data, the `restore --list` output, short proxy access-log retention; verify against
  the shipped behaviour.
- [ ] 13.5 Add README installation and upgrades (distribution: Installation and upgrades from
  release tags, Secrets and settings in the environment file); verify each statement they require.

## 14. Foundation acceptance

- [ ] 14.1 Real-browser check (foundation's part of design §12 Manual) under `just dev` or the smoke
  stack, by hand or a browser in Docker: no CSP violation on the landing page, same-origin form POST
  to `/` 405, cross-origin 403; record in the PR. The §12 login and report POSTs move to
  `admin-auth` and `report-submission`.
- [ ] 14.2 IPv6 check (design §12 Manual), by the owner if the agent has no global IPv6: dual-stack
  host, `KOHAKU_CADDY_CI`, `curl -6 --cacert … --resolve` from elsewhere; verify "IPv6 clients keep
  their address" and "Attacker forges X-Forwarded-For through Caddy" (301 requests from one client
  with rotating forged `X-Forwarded-For`: the last gets 429); record in the PR.
- [ ] 14.3 Owner only, agents skip this task: complete the one-time setup in `docs/releasing.md`
  before the first release tag (distribution: Owner-approved publish, Immutable version tags);
  verify each item against the guide.
- [ ] 14.4 Final check: `cargo fmt --check`, clippy, both `cargo test` builds, `cargo deny check`,
  `openspec validate --all --strict`, `just smoke` and `nix build` pass; every requirement maps to
  a passing test or recorded manual check.
