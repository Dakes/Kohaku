# Kohaku

**Bugs, preserved in amber.** (*kohaku*, 琥珀, is Japanese for amber.)

Kohaku is a tiny, self-hosted bug report inbox for small developers: apps, SaaS,
game mods. Each project gets a public report form, a public list of known bugs with
a permanent **Fixed** section, and a JSON API for in-app reporting. Reporters never
need an account.

> **Status: in design.** Nothing is runnable yet. The validated design is in
> [docs/plans/2026-09-23-kohaku-design.md](docs/plans/2026-09-23-kohaku-design.md);
> implementation proceeds as [OpenSpec](https://github.com/Fission-AI/OpenSpec)
> changes under [openspec/](openspec/).

## Features

- **Public, moderated reports.** New reports stay private until a maintainer approves
  them. Spam never goes public.
- **Simple lifecycle.** Pending → open → in progress → fixed, or closed (won't fix /
  duplicate) with a public reason. Fixed bugs stay listed forever. Any report can be
  hidden again, edited (audited) or deleted by the admin.
- **Markdown descriptions**, strictly sanitized (no raw HTML, no remote images).
- **Screenshots** (per project, off by default), re-encoded to WebP with all metadata
  stripped and stored inside the database, with per-project quotas.
- **"Me too" counter** so you can see which bugs hurt the most people (per project).
- **Optional reporter email verification** with a one-time code; verified reporters
  get notified when their bug is fixed.
- **Email notifications** for maintainers, batched so a spam wave is one mail, not 500.
- **Admin + invited maintainers** with per-project access, and TOTP two-factor login.
- **JSON API** for reporting from inside your app or mod.
- **Custom domains:** one instance can serve `bugs.pocetude.com` for one project and
  `bugs.reowls.de` for another, with HTTPS certificates obtained automatically.

## Security model

Kohaku accepts input from anyone on the internet, so it is built defensively:

- No reporter accounts. Proof-of-work, honeypot fields and per-IP rate limits stop
  bots without CAPTCHAs or third-party services.
- Server-rendered HTML with compile-time auto-escaping and a strict Content Security
  Policy (no inline scripts, no third-party anything).
- Uploaded images are decoded in memory-safe Rust under strict size and memory limits,
  then re-encoded; original bytes are never stored or served and never reach C code.
- Admin sessions: argon2id passwords, TOTP, `__Host-` cookies, CSRF tokens and
  Origin checks. Lockout after failed logins spares browsers you already logged in
  from, so nobody can lock you out of your own instance.
- A leaked database or backup alone yields no TOTP secrets: they are derived from a
  secret key that lives outside the database.
- Logs never contain emails, IP addresses, tokens or report text.
- Minimal, audited dependencies (`cargo deny` checks advisories, licenses, bans and
  sources in CI; advisories again in every release build); `unsafe` code is forbidden
  in Kohaku itself.
- Official images are published only after the maintainer approves each release, and
  are signed.

Found a vulnerability? Please report it privately via
[GitHub security advisories](https://github.com/Dakes/Kohaku/security/advisories/new),
not as a public issue.

## Deployment (planned)

Kohaku ships as a single static binary in a distroless Docker image (amd64 and arm64),
published to Docker Hub as `dakes/kohaku` for every release. Everything, screenshots
included, lives in one SQLite database in a single volume. **Docker Compose is the
supported way to run it**, and the repo ships a ready, hardened setup with Caddy for
automatic HTTPS. It needs Docker Engine 27 or newer.

First point your main domain's A/AAAA records at the server and open ports 80 and 443.

```sh
V=1.0.0                                       # the release you install
R=https://raw.githubusercontent.com/Dakes/Kohaku/refs/tags/$V
mkdir kohaku && cd kohaku
curl -fsSLO $R/compose.yaml
curl -fsSLO $R/Caddyfile
curl -fsSL  $R/.env.example -o .env

$EDITOR .env                                  # KOHAKU_DOMAIN, mail sender, SMTP server
mkdir -p secrets && chmod 700 secrets
$EDITOR secrets/smtp_password
head -c 32 /dev/urandom | base64 > secrets/kohaku_secret
chmod 644 secrets/*                           # see below

docker compose up -d
docker compose exec kohaku kohaku admin create --email you@example.org
```

Compose mounts secret files with their host owner and mode (it ignores `uid`, `gid`
and `mode` for them), so Kohaku, running as uid 65532, can only read them if they are
world-readable. The 0700 `secrets/` directory keeps every other host user out.

`admin create` prints a single-use setup link, valid for one hour. Open it to set your
password and enroll two-factor authentication; no password ever goes on the command
line.

The install files come from the release tag, not `main`. `docker compose pull` updates
the image but never these files; release notes say when they changed.

**Keep `secrets/kohaku_secret` safe and back it up separately.** Kohaku refuses to
start without it. If it is lost, create a new one and run `docker compose run --rm
kohaku kohaku admin rekey` with Kohaku stopped; everyone re-enrolls two-factor login.
A backup only starts with the secret it was made with.

**Verify the image** (optional) with [cosign](https://docs.sigstore.dev/): every
release is signed from the GitHub release workflow. The exact command is in
[docs/releasing.md](docs/releasing.md#verifying-an-image).

**Custom domains per project:** first point a DNS record (e.g. `bugs.example.org`) at
the server, then enter it in the project settings. Caddy gets the certificate on the
first visit; Kohaku tells it which domains are allowed, so nobody else can make your
server request certificates. Remove domains whose DNS no longer points at the server.
The admin area stays on your main domain only; ideally put it on a domain that no
project domain or other service shares. In-app clients should use the main-domain API
base `https://<main domain>/p/<slug>/api/v1`, which keeps working if a project's domain
changes.

What the shipped `compose.yaml` does for you:

- **Only Caddy is exposed** (ports 80/443). Kohaku sits on a private dual-stack
  network with Caddy and trusts forwarded client IPs from exactly Caddy's two fixed
  addresses. (It is not a Compose `internal: true` network, which would cut off mail
  and certificates.)
  The subnets, Caddy's addresses and `KOHAKU_TRUSTED_PROXIES` sit next to each other
  in `compose.yaml` and change together.
- **Hardened containers:** read-only root filesystems, all capabilities dropped (Caddy
  keeps only the right to bind 80/443), `no-new-privileges`, non-root Kohaku, memory
  limits, healthcheck, size-capped logs. Certificates live in named volumes.
- **Secrets as files** (`secrets/smtp_password`, `secrets/kohaku_secret`), never as
  environment variables visible in `docker inspect`.
- **Safe updates:** the image is pinned to the major version (`dakes/kohaku:1`), so
  `docker compose pull && docker compose up -d` brings fixes but never a breaking
  release. Migrations run automatically at startup, after Kohaku saves a copy of the
  database in `/data/backups/`.
- **No insecure defaults:** Kohaku refuses to start while a required setting is
  missing or still has its example value.

**Already running Traefik, nginx or another proxy?** Remove the `caddy` service, attach
`kohaku` to your proxy's network, set `KOHAKU_TRUSTED_PROXIES` to your proxy's exact
address, and make the proxy pass the client address. For nginx:

```nginx
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
```

(NixOS: `recommendedProxySettings = true;`). Never publish Kohaku's port beyond the
proxy network, and don't let the proxy or a CDN cache `/admin`, `/p`, `/api` or
screenshots. Proxy access logs contain one-time links and IP addresses; keep their
retention short.

**Backups:** one database file is the whole backup (keep `secrets/kohaku_secret`
alongside it, stored separately). Kohaku streams a consistent snapshot while running;
copy the file off the host:

```sh
(umask 077; docker compose exec -T kohaku kohaku backup - > kohaku-$(date +%F).db)
```

**Restore** with the service stopped:

```sh
docker compose stop kohaku
docker compose run --rm -T kohaku kohaku restore - < kohaku-2026-09-23.db
docker compose start kohaku
```

**Rolling back an update:** stop Kohaku, list the `pre-migrate-…` copies with
`docker compose run --rm kohaku kohaku restore --list`, restore one with
`docker compose run --rm kohaku kohaku restore /data/backups/pre-migrate-…`, pin the
previous exact version in `compose.yaml` (e.g. `dakes/kohaku:1.2.3`) and start it.
A newer database refuses to run on an older Kohaku. Erased data can survive in older
backups and volume snapshots.

`docker compose down -v` deletes all data and certificates.

Full configuration reference (SMTP, trusted proxies, limits) will follow with the
first release.

## Development

Two supported setups, both using the toolchain pinned in `rust-toolchain.toml`:

```sh
# Nix (flake devShell, includes all dev tools)
nix develop          # or: direnv allow

# Anywhere else
rustup toolchain install                 # the toolchain from rust-toolchain.toml
npm i -g @fission-ai/openspec@1.13.1     # the OpenSpec version the flake pins
```

Elsewhere, also install `just`, `watchexec-cli` and `cargo-deny` with
`cargo install --locked <crate>@=<version>` at the versions listed in
`docs/dependencies.md` (the ones the flake provides).

Specs come before code. To work on a feature:

```sh
openspec list        # changes in flight
openspec view        # dashboard of specs and changes
```

Contributor and AI-agent guidelines: [AGENTS.md](AGENTS.md).

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified Kohaku as a service for
others, you must offer them its source.
