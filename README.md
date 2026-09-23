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
  duplicate) with a public reason. Fixed bugs stay listed forever.
- **Markdown descriptions**, strictly sanitized (no raw HTML, no remote images).
- **Screenshots** (per project, off by default), re-encoded to WebP with all metadata
  stripped.
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
- Uploaded images are decoded in memory-safe Rust and re-encoded; original bytes are
  never stored or served.
- Admin sessions: argon2id passwords, TOTP, `__Host-` cookies with `SameSite=Strict`,
  CSRF tokens, account lockout.
- Logs never contain emails, IP addresses, tokens or report text.
- Minimal, audited dependencies (`cargo audit`, `cargo deny` in CI); `unsafe` code is
  forbidden in Kohaku itself.

Found a vulnerability? Please report it privately via
[GitHub security advisories](https://github.com/Dakes/Kohaku/security/advisories/new),
not as a public issue.

## Deployment (planned)

Kohaku ships as a single static binary in a distroless Docker image (amd64 and arm64),
published to Docker Hub for every release. Everything lives in one SQLite database
plus a media folder in a single volume. **Docker Compose is the supported way to run
it**, and the repo ships a ready, hardened setup with Caddy for automatic HTTPS.

```sh
mkdir kohaku && cd kohaku
curl -fsSLO https://raw.githubusercontent.com/Dakes/Kohaku/main/compose.yaml
curl -fsSLO https://raw.githubusercontent.com/Dakes/Kohaku/main/Caddyfile
curl -fsSL  https://raw.githubusercontent.com/Dakes/Kohaku/main/.env.example -o .env

$EDITOR .env                                  # domain, mail sender, SMTP server
mkdir -p secrets && $EDITOR secrets/smtp_password

docker compose up -d
docker compose exec kohaku kohaku admin create
```

Then open `https://<your domain>/admin`.

**Custom domains per project:** point a DNS record (e.g. `bugs.example.org`) at the
server and enter it in the project settings. Caddy gets the certificate on the first
visit; Kohaku tells it which domains are allowed, so nobody else can make your server
request certificates. The admin area stays on your main domain only.

What the shipped `compose.yaml` does for you:

- **Only Caddy is exposed** (ports 80/443). Kohaku sits on an internal network with a
  fixed subnet, so it trusts forwarded client IPs from exactly one known proxy.
- **Hardened container:** read-only root filesystem, all capabilities dropped,
  `no-new-privileges`, non-root user, memory limit, healthcheck.
- **Secrets as files** (`secrets/smtp_password`), never as environment variables
  visible in `docker inspect`.
- **Safe updates:** the image is pinned to the major version (`dakes/kohaku:1`), so
  `docker compose pull && docker compose up -d` brings fixes but never a breaking
  release. Migrations run automatically at startup.
- **No insecure defaults:** Kohaku refuses to start while a required setting is
  missing or still has its example value.

**Already running Traefik, nginx or another proxy?** Remove the `caddy` service, attach
`kohaku` to your proxy's network, and set `KOHAKU_TRUSTED_PROXIES` to that network's
subnet. The configuration reference will cover this.

**Backups:** `docker compose exec kohaku kohaku backup /data/backup.db` makes a
consistent snapshot while running. Copy it, plus `/data/media`, off the host.

Full configuration reference (SMTP, trusted proxies, limits) will follow with the
first release.

## Development

Two supported setups, both using the toolchain pinned in `rust-toolchain.toml`:

```sh
# Nix (flake devShell, includes all dev tools)
nix develop          # or: direnv allow

# Anywhere else
rustup show          # installs the pinned toolchain
```

Specs come before code. To work on a feature:

```sh
openspec list        # changes in flight
openspec view        # dashboard of specs and changes
```

Contributor and AI-agent guidelines: [AGENTS.md](AGENTS.md).

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified Kohaku as a service for
others, you must offer them its source.
