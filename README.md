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
- **Feature requests** (per project, off by default), moderated and listed like bugs.
- **Markdown descriptions**, strictly sanitized (no raw HTML, no remote images).
- **Screenshots** (per project, off by default), re-encoded to WebP with all metadata
  stripped and stored inside the database, with per-project quotas.
- **"+1" counter** so you can see which bugs hurt the most people (per project).
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
curl -fsSLO $R/docker-compose.yml
curl -fsSLO $R/Caddyfile
curl -fsSL  $R/.env.example -o .env
chmod 600 .env                                # it holds your secrets
mkdir data                                    # Kohaku runs as uid 65532 in the container:
docker run --rm -v "$PWD/data:/data" alpine:3 sh -c 'chown 65532:65532 /data && chmod 700 /data'

head -c 32 /dev/urandom | base64              # your new KOHAKU_SECRET
$EDITOR .env                                  # domain, KOHAKU_SECRET, mail sender, SMTP

docker compose up -d
docker compose exec kohaku kohaku admin create --email you@example.org
```

Kohaku sends mail through your provider's SMTP server (TLS required); it needs no mail
server of its own. Put the SMTP password in single quotes (`KOHAKU_SMTP_PASSWORD='…'`)
so Compose takes it literally. Both secrets are visible to anyone who can run `docker
inspect` on the host, which only root and the `docker` group can, and they control the
host anyway.

`admin create` prints a single-use setup link, valid for one hour. Open it to set your
password and enroll two-factor authentication; no password ever goes on the command
line.

The install files come from the release tag, not `main`. `docker compose pull` updates
the image but never these files; release notes say when they changed.

**Keep `KOHAKU_SECRET` safe and back it up separately** (e.g. in a password manager).
Kohaku refuses to start without it. If it is lost, create a new one and run `docker compose run --rm
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

What the shipped `docker-compose.yml` does for you:

- **Only Caddy is exposed** (ports 80/443). Kohaku sits on a private dual-stack
  network that only it and Caddy join, and trusts forwarded client IPs only from that
  network. (It is not a Compose `internal: true` network, which would cut off mail and
  certificates.) The subnets and `KOHAKU_TRUSTED_PROXIES` sit next to each other in
  `docker-compose.yml` and change together.
- **Data beside the file:** Kohaku's database and backups in `./data`, Caddy's
  certificates in `./caddy`.
- **Hardened containers:** read-only root filesystems, all capabilities dropped (Caddy
  keeps only the right to bind 80/443), `no-new-privileges`, non-root Kohaku, memory
  limits, healthcheck, size-capped logs. Certificates live in named volumes.
- **No empty secrets:** `docker compose up` stops with a hint while `KOHAKU_SECRET` or
  `KOHAKU_SMTP_PASSWORD` is empty.
- **Safe updates:** the image is pinned to the major version (e.g. `dakes/kohaku:1`), so
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

**Backups:** one database file is the whole backup (keep `KOHAKU_SECRET` alongside it,
stored separately). Kohaku streams a consistent snapshot while running;
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
previous exact version in `docker-compose.yml` (e.g. `dakes/kohaku:1.2.3`) and start it.
A newer database refuses to run on an older Kohaku. Erased data can survive in older
backups and volume snapshots.

`restore --list` prints the file names in `/data/backups`, one per line: the
`pre-migrate-v{version}-{unixtime}.db` copies Kohaku keeps (the two newest) and any backups
you wrote there. Every backup and pre-migration copy keeps whatever was deleted after it
was made, until you delete that file.

Caddy publishes ports 80 and 443; set `KOHAKU_HTTP_PORT` and `KOHAKU_HTTPS_PORT` in `.env`
to use others (rootless Docker cannot publish ports below 1024).

Everything lives in `./data` (database, backups) and `./caddy` (certificates) next to
`docker-compose.yml`; back up or delete those folders to back up or delete the instance.

## Configuration

Kohaku reads its settings only from environment variables (with Compose: `.env`). There
is no configuration file and no setting on the command line. Values are used exactly as
written, never trimmed or corrected. On a problem Kohaku lists every broken setting and
exits before touching the data directory. A required setting set to the empty string
counts as missing.

| Setting | | Format |
|---|---|---|
| `KOHAKU_BASE_URL` | required | `https://host` or `https://host:port`: lowercase DNS name (punycode for international names), no IP address, no path or trailing `/`, port not 443. Compose builds it from `KOHAKU_DOMAIN`. |
| `KOHAKU_TRUSTED_PROXIES` | required | `none`, or a comma-separated list without spaces of your reverse proxy's **exact addresses** (e.g. `10.231.7.2,fd4b:7a1c:2e90:1::2`, set by Compose). CIDR blocks such as `10.231.7.0/29` only as a last resort: every address in the block may claim any client address. |
| `KOHAKU_SECRET` | required, secret | Standard base64 with `=` padding of at least 32 random bytes: `head -c 32 /dev/urandom \| base64`. |
| `KOHAKU_SMTP_HOST` | required | Your provider's SMTP server name (no IP address, port or scheme). Its certificate is verified. |
| `KOHAKU_SMTP_PORT` | required | 1-65535, usually 587 (`starttls`) or 465 (`implicit`). |
| `KOHAKU_SMTP_TLS` | required | `starttls` or `implicit`. Mail is never sent without verified TLS. |
| `KOHAKU_SMTP_USERNAME` | required | No control characters. |
| `KOHAKU_SMTP_PASSWORD` | required, secret | No control characters. |
| `KOHAKU_SMTP_FROM` | required | Sender address, bare `name@domain` (no display name). |
| `KOHAKU_PUBLIC_MAIL_PER_HOUR` | optional, default `60` | Mails per hour that unauthenticated visitors may cause, instance-wide. 1-4294967295, no leading zeros. Raise it if your SMTP provider allows more. |

Development builds (`just dev`) print mail instead of sending it: there, only
`KOHAKU_SMTP_FROM` is read and the other SMTP settings must be unset.

## Development

Two supported setups, both using the toolchain pinned in `rust-toolchain.toml`:

```sh
# Nix (flake devShell, includes all dev tools)
nix develop -c $SHELL   # your own shell (zsh, fish, ...) with the dev tools
direnv allow            # or: load them automatically on cd (direnv + nix-direnv)

# Anywhere else
rustup toolchain install                 # the toolchain from rust-toolchain.toml
npm i -g @fission-ai/openspec@1.13.1     # the OpenSpec version the flake pins
```

Elsewhere, also install `just`, `watchexec-cli` and `cargo-deny` with
`cargo install --locked <crate>@=<version>` at the versions listed in
`docs/dependencies.md` (the ones the flake provides).

`just dev` runs Kohaku on http://localhost:8080 and reloads open pages after every
change. It needs no mail server: development builds print every mail to the terminal
instead of sending it.

Specs come before code. To work on a feature:

```sh
openspec list        # changes in flight
openspec view        # dashboard of specs and changes
```

Contributor and AI-agent guidelines: [AGENTS.md](AGENTS.md).

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified Kohaku as a service for
others, you must offer them its source.
