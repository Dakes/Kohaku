# AGENTS

Guidelines for anyone (human or AI agent) changing this repo.

## Standard

Kohaku accepts untrusted input from the whole internet and is published as open
source. Everything you build must meet a high standard: bug free, tested, no
temporary changes, no workarounds, no shortcuts, no fallbacks. Do it right the first
time, even if it takes more effort.

**No implicit defaults**, especially for secrets or anything that affects security.
Fail at startup with a clear error so the operator notices, rather than silently
running with an insecure fallback. The same applies elsewhere: implicit defaults lead
to hard-to-debug behavior and dead code.

If something is unclear, don't guess and go ahead. Ask.

## Principles, in priority order

1. Security first.
2. Minimal resources.
3. Minimal dependencies. Every crate needs a one-line justification in
   `docs/dependencies.md`. Prefer a few lines of our own code over a crate for small
   things.
4. Boring, auditable code. No cleverness a reviewer has to puzzle over.

## Spec-driven development

Work goes through OpenSpec: propose (`/opsx:propose`), review, apply (`/opsx:apply`),
archive (`/opsx:archive`). No feature code without an accepted change. If
implementation reveals the spec is wrong, update the change first, then the code.

`docs/plans/2026-09-23-kohaku-design.md` is the design baseline until its content is
captured in `openspec/specs/`.

## Core rule

A feature change updates the full path together: migration, queries, handler,
template, config, docs and tests. Don't assume a change is "just UI" or "just logic"
until you've checked the whole path.

## Change checklists

**New route**
- Admin routes go through `RequireAdmin` or `ProjectAccess<Cap>`. Anything acting on a
  report, note or screenshot is nested under `/admin/p/{slug}/r/{number}/…` and binds
  the project_id from the guard; never look up a child by global id alone.
- Register it on the right host router (main or project); project hosts never set
  cookies.
- Add it to the route security matrix test. The test fails if you don't.
- State-changing routes are POST, public ones included, and pass the Fetch-Metadata /
  Origin middleware; admin POSTs also carry the CSRF token. No state changes on GET,
  including token links: GET renders, POST consumes. Only JSON API writes and the
  RFC 8058 one-click unsubscribe POST (authorized by its token) may arrive with
  neither `Origin` nor `Sec-Fetch-Site`.
- Give it a Cache-Control class and a body cap (64 KiB unless the design says
  otherwise).

**New public input**
- Size limit, rate limit class, validation, and an abuse test. Cheap checks run
  before the body is read.
- Rendered only through askama escaping or the Markdown sanitizer, never `|safe` on
  user data.
- Public reads go only through the public SQLite views and `Public*` structs.

**Database change**
- One migration per change: while the change is unmerged, edit its migration instead
  of adding another. Never edit a migration that has shipped in a release.
- Migration, query code and tests in the same change.
- Every check-and-consume or check-and-count (tokens, codes, attempts, caps) is one
  conditional statement or transaction on the writer, decided by rows affected or
  `RETURNING`. Never decide security from a reader snapshot.

**New admin action or CLI command**
- Call `audit()` with ids only (actor `cli` for the CLI). Never emails, titles, bodies
  or IPs.

**New config setting**
- Required settings have no default: startup fails with a clear message if missing.
- Update the example config and the configuration section of the README.
- Security behaviour belongs in the image, not in `compose.yaml` or the `Caddyfile`
  (`docker compose pull` never updates those).

**New key or MAC**
- Add it to the design's key inventory. Prefer a per-boot key or a random value stored
  hashed; derive from the instance secret (`KOHAKU_SECRET`) only when it must
  survive restarts, and make `admin rekey` reset what depends on it. Purpose label
  first, length-prefixed fields, `verify_slice`.

**New email**
- Plain text only, sent through the outbox, never inline in a request.
- Token-bearing mail is deleted from the outbox when sent or given up. Maintainer mail
  carries no user-supplied content.
- User-supplied text in a subject or body must be single-line, length-capped, and
  marked as quoted.

**New dependency**
- Justify it in `docs/dependencies.md`; `cargo deny check` must pass. Keep default
  features off unless needed. A new build script must be added to
  `allow-build-scripts` deliberately.
- libwebp is used through the encoder only; its decoder APIs are banned in
  `clippy.toml` and by a test (design §6 Screenshots).

## Logging

Never log raw IPs, email addresses, tokens, auth headers, query strings or report
text. A test asserts this. When debugging needs more, log opaque IDs.

## Comments

Inline comments: 2 lines max, more only in exceptional cases. Explain the *why* the
code can't show (a gotcha, a constraint, a rejected alternative). Don't restate what
the code already says.

Does not apply to doc comments (`///`).

The same goes for this file and other docs: top-level info only, nothing a reader
would learn from the files themselves.

## Development environment

The maintainer's host runs **NixOS**; agents usually run in a non-Nix sandbox. Both
must work: the toolchain is pinned once in `rust-toolchain.toml` and used by both the
Nix flake devShell and rustup. A new dev tool goes into the flake **and** is
installable with cargo/rustup (or an exact pinned version otherwise). OpenSpec comes
from nixpkgs in the flake; elsewhere `npm i -g @fission-ai/openspec@1.13.1`. No
nodejs in the devShell. Anything needing a browser runs in Docker.

Dev-only code lives behind the `dev` feature, which must never reach a release build
(a `compile_error!` enforces it). Never loosen the release CSP or headers for dev
convenience; the dev CSP only adds `connect-src 'self'`.

## Releases

Git tags are bare semver: `1.0.0`, `1.0.1` (no `v` prefix). The tag must equal the
version in `Cargo.toml`; CI fails otherwise. Pushing a tag starts the release; the
image is published only after the owner approves the deployment in the `release`
environment. Agents never push tags, never touch release settings, and never change
`.github/workflows/` to reference the `release` environment from another job. Steps:
[docs/releasing.md](docs/releasing.md).

## Commits

In interactive sessions, never commit: leave the work in the working tree and the
maintainer commits. Autonomous agents may commit.

## Publishing

Never publish anything to claude.ai (e.g. via the Artifact tool) unless the maintainer
explicitly asks for it in that conversation.

## Before finishing a change

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --all-features
cargo deny check
openspec validate --all --strict
```

If the change affects pages or emails, also check them by hand with `just dev`, which
prints every mail to the terminal instead of sending it.
