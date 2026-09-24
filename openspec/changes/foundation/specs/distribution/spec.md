# Spec Delta

## Purpose

How Kohaku reaches operators: a `dev` feature that never ships, pinned toolchains, secret-free CI,
the signed owner-approved release, the image and the hardened Compose deployment (design §11, §13, §14).

## ADDED Requirements

### Requirement: Development feature excluded from release builds

Development-only behaviour SHALL exist only in `dev`-feature builds. Enabling `dev` without debug
assertions (any release-profile build) MUST fail compilation with an error stating that the dev
feature is in a release build; a debug build with `dev` compiles. Published binaries and images
SHALL use default features only; a build without `dev` SHALL serve no development route on any host.

#### Scenario: Attacker probes a release build for development routes

- **WHEN** an attacker requests `/dev/boot-id` and `/static/dev-reload.js` on the main host of a build without `dev`
- **THEN** both are 404 and the landing page references no reload script

### Requirement: Development live reload

`just dev` SHALL rebuild and restart the `dev` server on every project source change, after which
open HTML pages SHALL reload themselves. A `dev` build SHALL:

- serve `/static/dev-reload.js` and `/dev/boot-id` on the main host and every project host, and
  load the former in every HTML page as an external script that polls the latter on the page's
  origin once per second, keeps polling through failures, and reloads once the identifier differs
  from the first one it got;
- give `/dev/boot-id` an identifier constant for the process lifetime and new after every restart;
- read stylesheets and scripts from the source tree, not the embedded copies;
- use the `http-security` release CSP plus `connect-src 'self'`, with no inline script,
  `'unsafe-inline'`, `'unsafe-eval'`, nonces or hashes, and every other security header and
  `Cache-Control` equal to a build without `dev`;
- use `./data`, relative to the working directory, wherever other capabilities name `/data`.

#### Scenario: Page reloads after an edit

- **WHEN** a page under `just dev` is open and a Rust source, template or stylesheet is edited, or the server is down for 60 seconds and restarts
- **THEN** polling continues once per second and the page reloads within 1 second after the restarted server first answers `/dev/boot-id` (Rust recompiles only for Rust or template edits)

#### Scenario: Attacker-injected script stays blocked in development

- **WHEN** an attacker gets an inline `<script>`, or a script fetching another origin, into a page of a `dev` build
- **THEN** the browser refuses both

### Requirement: Pinned toolchain and development environments

`rust-toolchain.toml` SHALL pin the toolchain once, to an exact `1.NN.P` no older than 1.89 with
the `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` targets, clippy and rustfmt, and
the Nix devShell, the flake's default package and rustup SHALL all use exactly it (never nixpkgs
`rustc`). The devShell SHALL provide just, watchexec, cargo-deny, cargo-zigbuild, zig, sqlite
and OpenSpec 1.13.1 from nixpkgs pinned by `flake.lock`, and no nodejs, npm or mail server;
`.envrc` SHALL contain `use flake`. Outside Nix, just, watchexec-cli and cargo-deny SHALL install
with `cargo install --locked` at the flake's exact versions, recorded in `docs/dependencies.md`.
Development needs no mail server or container: a `dev` build prints mail (mail-outbox).

#### Scenario: Same toolchain with and without Nix

- **WHEN** `rustc --version` runs in `nix develop` and in a rustup setup of the same checkout, and `just dev` starts in each
- **THEN** both print the pinned version, `node` and `npm` are absent from the devShell, and `just dev` serves the landing page without Docker or an SMTP server

### Requirement: Continuous integration without secrets

CI SHALL run on every push and `pull_request` event with a `contents: read` token and no secrets;
no workflow SHALL trigger on `pull_request_target` or `workflow_run`. Every job SHALL declare
least-privilege `permissions`, every external action SHALL be pinned by full 40-character commit
SHA, every CI and release cargo invocation SHALL pass `--locked`, and no CI or release job SHALL
install nodejs or OpenSpec or run OpenSpec. A run SHALL fail unless all pass: `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` with default features,
`cargo test --all-features`, `cargo deny check` (advisories, licenses, bans, sources), a static
musl build, the Docker image build and the Compose smoke test.

#### Scenario: Attacker's pull request runs without secrets

- **WHEN** an attacker's fork pull request tries to read secrets, request an OIDC token or push to the repository
- **THEN** its token only reads contents, and no repository or environment secret or OIDC token is available

### Requirement: Dependency policy enforced in CI

`cargo deny check`, and with it CI, SHALL fail when the tree contains openssl, openssl-sys,
native-tls, aws-lc-rs, aws-lc-sys, rayon, rav1e or ravif; a crate with a build script missing from
the allow-list recorded at adoption; a crate from git or a registry other than crates.io; or a
crate version in `Cargo.lock` with a RustSec advisory. Dependabot SHALL propose grouped monthly
updates for `github-actions`, `docker` and `cargo`. An advisory affecting a crate in a released
binary SHALL be answered with a patch release.

#### Scenario: Attacker adds a banned crate

- **WHEN** an attacker's pull request pulls in `openssl-sys` or another banned crate, an unlisted build script, or a git or non-crates.io source, or an advisory hits `Cargo.lock`
- **THEN** `cargo deny check` and the CI run fail

### Requirement: Compose smoke test in CI

Every CI run SHALL build the image with the release Dockerfile from its own static musl binary and
run the shipped `docker-compose.yml` and `Caddyfile` (only the kohaku image reference changed) with
`KOHAKU_CADDY_CI` selecting Caddy's local CA and no GitHub secret, failing unless each step passes:

- `docker compose config`, then starting on an empty `./data` prepared as the README says until
  kohaku is healthy;
- through Caddy, trusting only the root certificate in Caddy's `/data` and resolving names to the
  runner: the main host's `/` is the landing page (`host-routing`) with 200, and the TLS handshake
  for a name Kohaku does not serve fails;
- `kohaku backup -` via `docker compose exec -T` writes a SQLite database to stdout; with kohaku
  stopped, `kohaku restore -` of it via `docker compose run --rm -T` exits 0, and kohaku then
  becomes healthy again.

#### Scenario: ENTRYPOINT regression fails CI

- **WHEN** the image under test declares an ENTRYPOINT, or the shipped `docker-compose.yml` fails `docker compose config`
- **THEN** that step fails and so does the run

### Requirement: Release build from a matching version tag

`.github/workflows/release.yml` SHALL build and publish only for a pushed git tag of exactly
`X.Y.Z` (three dot-separated decimal numbers, no prefix or suffix), failing before any build when
the tag has another form or differs from the `version` in `Cargo.toml`.

- The build job SHALL fail before compiling unless `cargo deny check advisories` passes, build
  static musl x86_64 and aarch64 binaries with `--locked` and the release profile using a build
  tool and cross linker at exact pinned versions, and upload each with its SHA-256; it SHALL have
  `contents: read` only, no secrets, no `id-token`, no git credentials in the checkout and no
  restored cache.
- Publish SHALL depend on it and on a job running the aarch64 binary with `--version` on a native
  arm64 runner.

#### Scenario: Tag not of the form X.Y.Z

- **WHEN** tag `v1.2.3`, `1.2.3-rc1` or `1.2`, or `1.2.4` on a commit whose `Cargo.toml` says `1.2.3`, is pushed
- **THEN** the run fails before any binary is built and nothing is published

#### Scenario: Attacker code in the build finds no credentials

- **WHEN** an attacker's code runs in the release build as a dependency's build script or procedural macro
- **THEN** it finds no Docker Hub or git credential, cannot get an OIDC token, and no cache from another run was restored

### Requirement: Owner-approved publish

Only the release workflow's publish job SHALL reference the GitHub environment `release`, and it
SHALL NOT start before the repository owner approves the deployment.

- `release` SHALL have the owner as required reviewer with administrator bypass disabled, accept
  deployments only from tags matching `[0-9]*.[0-9]*.[0-9]*`, and be the only place holding
  `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`; no repository or organization secret SHALL hold a
  Docker Hub credential.
- The publish job SHALL have exactly `contents: read` and `id-token: write`, and SHALL verify every
  binary against the build job's SHA-256 before logging in to Docker Hub.
- The Docker Hub token SHALL be Read & Write without Delete, expiring at most 1 year after creation.

#### Scenario: Attacker with tag-push access cannot publish

- **WHEN** a valid release tag is pushed, also by an attacker who can push tags but is not the owner
- **THEN** publish waits for review, and nothing reads the Docker Hub credentials or pushes until the owner approves

### Requirement: Signed multi-arch image

The publish job SHALL push one `linux/amd64` + `linux/arm64` image index to
`docker.io/dakes/kohaku` by digest, with provenance attestations and without CPU emulation; sign
that digest keyless with cosign 3.x as a Sigstore bundle stored as an OCI referrer; and only then
tag it `X.Y`, `X`, `latest`, and `X.Y.Z` last. No tag SHALL ever point at an unsigned digest, and a
re-run after a failure before `X.Y.Z` was applied SHALL complete. An SBOM attestation, if
published, MUST list the Rust crates from `Cargo.lock`; one listing no crates SHALL NOT be
published. For every published release this check SHALL succeed with cosign 3.0 or newer:

```sh
cosign verify \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/Dakes/Kohaku/\.github/workflows/release\.yml@refs/tags/[0-9]+\.[0-9]+\.[0-9]+$' \
  dakes/kohaku:X.Y.Z
```

#### Scenario: Attacker's signature does not verify

- **WHEN** the check above runs against an image signed from another repository, another workflow file or a branch ref
- **THEN** it fails, while for `dakes/kohaku:X.Y.Z` and `dakes/kohaku:X` it exits 0 and reports the digest

### Requirement: Immutable version tags

Release tags SHALL be immutable: a GitHub tag ruleset for `[0-9]*.[0-9]*.[0-9]*` SHALL block
updates and deletions with no bypass actor, and Docker Hub SHALL treat tags matching
`^[0-9]+\.[0-9]+\.[0-9]+$` as immutable while `X.Y`, `X` and `latest` stay mutable. A faulty
release SHALL be fixed only by the next patch version. Releases SHALL come only from the newest
version line (no backports to an older `X.Y`), and publish SHALL always move `X.Y`, `X` and
`latest` to the new digest.

#### Scenario: Attacker cannot move or delete a release tag

- **WHEN** anyone, the owner included, force-pushes or deletes git tag `1.2.3`, or a holder of the Docker Hub token pushes to an existing `X.Y.Z`
- **THEN** GitHub or Docker Hub rejects it and the tag keeps its target

### Requirement: Container image contract

The image SHALL be the digest-pinned `gcr.io/distroless/static-debianNN:nonroot` base plus exactly
the statically linked `kohaku` binary at `/usr/local/bin/kohaku` and a `/data` directory owned by
65532:65532, mode 0700, containing only `.keep` (a new named volume there inherits both). It SHALL
run as uid and gid 65532 with `kohaku` on `PATH`, declare `CMD ["kohaku", "serve"]` and no
ENTRYPOINT, and contain no shell. Its Dockerfile SHALL reference the base by `@sha256:` and add
content with COPY only (no RUN); CI and release SHALL build from that same Dockerfile.

#### Scenario: Attacker with code execution finds no shell

- **WHEN** an attacker with code execution in the kohaku process tries to start `/bin/sh` or any other shell
- **THEN** no shell exists and the process is not root

### Requirement: Hardened Kohaku service in Compose

The shipped `docker-compose.yml` SHALL run `kohaku` from `dakes/kohaku:X` (`X` = the major version of
the release it ships with) with no published host port, `read_only: true`, `cap_drop: [ALL]`,
`security_opt: [no-new-privileges:true]`, a tmpfs at `/tmp`, no `user` override, the named volume
the bind mount `./data` (beside `docker-compose.yml`) at `/data`, healthcheck `["CMD", "kohaku", "healthcheck"]` (no shell),
`mem_limit: 640m`, `restart: unless-stopped`, `KOHAKU_BASE_URL: https://${KOHAKU_DOMAIN:?}` (so
Compose fails naming `KOHAKU_DOMAIN` when it is unset or empty), and bounded logs: the `local`
driver, or `json-file` with `max-size: 10m` and `max-file: 3`.

#### Scenario: Attacker cannot reach Kohaku directly

- **WHEN** an attacker connects to the Docker host on 8080 or any port other than Caddy's 80 and 443
- **THEN** no connection reaches Kohaku

#### Scenario: Compromised process is confined

- **WHEN** an attacker with code execution in kohaku tries to write outside `/data` and `/tmp`, use a capability, or gain privileges via setuid
- **THEN** each attempt fails

### Requirement: Hardened Caddy reverse proxy

TLS SHALL terminate in a `caddy` service, the only service publishing host ports, exactly 80 and
443. Neither `docker-compose.yml` nor the `Caddyfile` SHALL set or remove security headers (no `header`
directive) or impose request limits or validation; those ship in the image.

- caddy SHALL use an explicit `caddy:2.N` tag or digest (the smoke test runs the same reference),
  kohaku's `cap_drop`, `security_opt`, `read_only`, `/tmp` tmpfs, `restart` and log settings plus
  `cap_add: [NET_BIND_SERVICE]`, `mem_limit: 256m`, the bind mounts `./caddy/data:/data` and
  `./caddy/config:/config` (certificates and ACME account survive `down`), and the
  environment `KOHAKU_DOMAIN: ${KOHAKU_DOMAIN:?}` and `KOHAKU_CADDY_CI: ${KOHAKU_CADDY_CI:-}`;
  with `KOHAKU_CADDY_CI` unset there SHALL be no local CA, only Caddy's default public ACME issuers.
- The `Caddyfile` SHALL have a global block with `{$KOHAKU_CADDY_CI}` and on-demand TLS asking
  `http://kohaku:8080/.well-known/kohaku/tls-ask`, a `{$KOHAKU_DOMAIN}` site and a catch-all
  `https://` site with on-demand TLS; both sites SHALL proxy to `kohaku:8080` and themselves answer
  404 for every path under `/.well-known/kohaku/`.

#### Scenario: Attacker's domain gets no certificate

- **WHEN** an attacker points DNS for a name that is neither the main domain nor a project host at the server and connects
- **THEN** tls-ask answers 404, Caddy requests no certificate from any issuer, fails the handshake and holds no certificate for the name

#### Scenario: Attacker cannot reach tls-ask from outside

- **WHEN** an attacker requests `https://{KOHAKU_DOMAIN}/.well-known/kohaku/tls-ask?domain=example.org` or any path under `/.well-known/kohaku/`
- **THEN** Caddy answers 404 without forwarding to Kohaku

### Requirement: Private dual-stack proxy network

`docker-compose.yml` SHALL define one network, `proxy`: `enable_ipv6: true`, not `internal`, a
fixed IPv4 /29 inside 10.0.0.0/8 but outside 10.0.0.0/16, and a fixed IPv6 /64 inside fd00::/8
with a non-zero 40-bit global ID. Both services, and nothing else, SHALL attach only to it. The
kohaku `environment:` block SHALL set `KOHAKU_TRUSTED_PROXIES` to exactly those two subnets as a
literal next to them, never interpolated from `.env` or the shell. The deployment SHALL require
Docker Engine 27 or newer.

#### Scenario: Environment cannot widen the trust list

- **WHEN** `.env` or the invoking shell defines `KOHAKU_TRUSTED_PROXIES`
- **THEN** kohaku still receives exactly the network's two subnets

#### Scenario: Attacker forges X-Forwarded-For through Caddy

- **WHEN** an attacker sends requests through Caddy carrying forged `X-Forwarded-For` values
- **THEN** Kohaku resolves each to the attacker's own address, so their `request-limits` rate limits apply

#### Scenario: IPv6 clients keep their address

- **WHEN** an IPv6 client reaches the main host on a dual-stack Docker Engine 27 host (checked once by hand for `foundation`)
- **THEN** Kohaku resolves its public IPv6 address, not a gateway, and logs no `client address is loopback/RFC 1918/ULA/link-local` warning

### Requirement: Secrets and settings in the environment file

The instance secret and the SMTP password SHALL reach kohaku as `KOHAKU_SECRET` and
`KOHAKU_SMTP_PASSWORD`, interpolated by `docker-compose.yml` from `.env` like the other operator
settings, each as `${NAME:?message}` whose message says how to set it, so `docker compose up`
stops while either is unset or empty. No secret value SHALL appear in `docker-compose.yml` or
`.env.example`. git SHALL ignore `.env` and track `.env.example`.

- `.env.example` SHALL list every required variable `docker-compose.yml` does not fix: `KOHAKU_DOMAIN`
  (single source of the main domain for both services), `KOHAKU_SECRET` and
  `KOHAKU_SMTP_PASSWORD` left empty, and the `configuration` mail sender and SMTP server
  settings, with `KOHAKU_DOMAIN` (via the base URL built from it), `KOHAKU_SMTP_HOST`,
  `KOHAKU_SMTP_USERNAME` and `KOHAKU_SMTP_FROM` as placeholders `configuration` rejects as
  unchanged, `KOHAKU_SMTP_PORT` and `KOHAKU_SMTP_TLS` optionally `587` and `starttls`, and no
  `KOHAKU_TRUSTED_PROXIES` or `KOHAKU_CADDY_CI`.
- The README SHALL say to `chmod 600 .env`; to generate `KOHAKU_SECRET` as 32 random bytes in
  base64; to single-quote the SMTP password so Compose takes it literally; to keep a copy of
  `KOHAKU_SECRET` apart from the database backups, since a backup starts only with the secret it
  was made with; and that anyone who can run `docker inspect` on the host can read both secrets.

#### Scenario: Unedited template never runs

- **WHEN** the stack starts with an unedited `.env.example` as `.env`, and again with only the two secrets filled in
- **THEN** the first `docker compose up` fails naming an empty secret variable before any container starts; in the second kohaku exits at startup and never reports healthy

#### Scenario: Secrets stay out of the repository and the shipped files

- **WHEN** an operator fills in `.env` in a clone as the README says, runs `git status`, and an attacker reads the shipped `docker-compose.yml` and `.env.example`
- **THEN** git reports no change and neither secret value appears in the shipped files

### Requirement: Installation and upgrades from release tags

The README SHALL install by fetching `docker-compose.yml`, `Caddyfile` and `.env.example` from
`https://raw.githubusercontent.com/Dakes/Kohaku/refs/tags/X.Y.Z/` for the current release, never a
branch; the `docker-compose.yml` at tag `X.Y.Z` SHALL pin `dakes/kohaku:X`, so `docker compose pull`
delivers the newest release of that major version and never a newer major.

- Release notes SHALL name every change to `docker-compose.yml`, `Caddyfile` or `.env.example` since the
  previous release.
- The README SHALL state: Docker Engine 27 or newer is required; all data and certificates
  live in `./data` and `./caddy` beside `docker-compose.yml`; `./data` must be created first
  owned by 65532:65532 with mode 0700; the subnets and `KOHAKU_TRUSTED_PROXIES` change together; and, behind an existing proxy, remove `caddy`, attach
  `kohaku` to that proxy's network, set `KOHAKU_TRUSTED_PROXIES` to the proxy's exact address, make
  the proxy set `X-Forwarded-For`, and never publish Kohaku's port beyond the proxy network.

#### Scenario: Attacker's branch is never fetched

- **WHEN** an attacker gets a modified `docker-compose.yml` onto `main` or onto a branch named `X.Y.Z`
- **THEN** the README's `refs/tags/X.Y.Z/` URLs still return the tag's files

#### Scenario: Pull stays within the major version

- **WHEN** an operator with `docker-compose.yml` from `1.2.3` runs `docker compose pull && docker compose up -d` after `1.4.0` and `2.0.0` exist
- **THEN** kohaku runs `1.4.0`
