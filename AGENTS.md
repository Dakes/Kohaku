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
- Admin routes go through the `ProjectAccess` extractor (or the admin-only guard).
- Add it to the route security matrix test. The test fails if you don't.
- State-changing routes are POST with a CSRF token. No state changes on GET.

**New public input**
- Size limit, rate limit class, validation, and an abuse test.
- Rendered only through askama escaping or the Markdown sanitizer, never `|safe` on
  user data.

**Database change**
- One migration per change: while the change is unmerged, edit its migration instead
  of adding another. Never edit a migration that has shipped in a release.
- Migration, query code and tests in the same change.

**New config setting**
- Required settings have no default: startup fails with a clear message if missing.
- Update the example config and the configuration section of the README.

**New email**
- Plain text only, sent through the outbox, never inline in a request.
- User-supplied text in a subject or body must be single-line, length-capped, and
  marked as quoted.

**New dependency**
- Justify it in `docs/dependencies.md`; `cargo deny check` must pass. Keep default
  features off unless needed.

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
installable with cargo/rustup. Anything needing a browser runs in Docker.

## Releases

Git tags are bare semver: `1.0.0`, `1.0.1` (no `v` prefix). Pushing a tag builds and
publishes the Docker image. The tag must equal the version in `Cargo.toml`; CI fails
otherwise.

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
cargo test --all-features
cargo deny check
cargo audit
openspec validate --strict
```

If the change affects pages or emails, also check them by hand (`just dev`, Mailpit).
