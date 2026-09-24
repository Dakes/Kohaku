# Dependencies

Every crate, tool and image Kohaku depends on, pinned, with the reason it is needed.
Review this file on every bump (AGENTS.md: New dependency).

## Toolchain

Rust **1.98.1**, pinned in `rust-toolchain.toml` with clippy, rustfmt and the targets
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. rustup reads the file
directly; the Nix flake reads the same file through rust-overlay.

## System prerequisites outside Nix

Debian/Ubuntu: `sudo apt install build-essential file`.

- `cc` (build-essential): compiles the bundled SQLite (`rusqlite` `bundled`).
- `file`: checks that release binaries are statically linked.

## Development and CI tools

The Nix devShell provides these from the nixpkgs revision in `flake.lock`. Outside Nix,
install the same versions with
`cargo install --locked just@=1.58.0 watchexec-cli@=2.5.1 cargo-deny@=0.20.2`.
Bump these pins together with `flake.lock`.

| Tool | Version | Why |
|---|---|---|
| just | 1.58.0 | Task runner (`just dev`, `just smoke`) |
| watchexec-cli | 2.5.1 | Restarts the dev server on source changes (`just dev`) |
| cargo-deny | 0.20.2 | Advisories, licenses, bans and sources in CI |
| cargo-zigbuild | 0.23.4 | Static musl builds for both architectures (CI and release, `cargo install --locked`) |
| zig | 0.16.0 | Cross linker for cargo-zigbuild; CI downloads the official tarball and checks its SHA-256 |
| OpenSpec | 1.13.1 | Spec workflow; nixpkgs, elsewhere `npm i -g @fission-ai/openspec@1.13.1`; never in CI |
| sqlite | nixpkgs | `sqlite3` shell for inspecting databases by hand |

zig 0.16.0 tarballs (from `https://ziglang.org/download/0.16.0/`):

- `zig-x86_64-linux-0.16.0.tar.xz`
  SHA-256 `70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`
- `zig-aarch64-linux-0.16.0.tar.xz`
  SHA-256 `ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17`

## Crates

All direct dependencies use `default-features = false`, with exactly the features below.
Licenses and build scripts are allowed in `deny.toml` exactly as the tree needed them at
adoption (7 licenses, 13 build scripts); a new one fails CI until someone accepts it there.
"Resolved" is what the whole tree turns on (`cargo tree -e features`, x86_64 musl).

| Crate | Declared features | Resolved | Why |
|---|---|---|---|
| askama 0.16.1 | `derive`, `std` | + `alloc` | Compile-time HTML templates, escaping on by default |
| axum 0.8.9 | `http1`, `tokio`, `matched-path` | same | Routing and extractors; `matched-path` gives tracing the route pattern |
| base64 0.23.1 | `std` | + `alloc`, `default`, `simd-unsafe` (from lettre) | Strict padded decoding of `KOHAKU_SECRET` |
| getrandom 0.4.3 | none | none | The OS random source: per-boot keys, tokens, temporary names |
| hmac 0.13.0 | none | none | HMAC-SHA256 for every derived value, `verify_slice` in constant time |
| hyper-util 0.1.20 | `tokio`, `server`, `http1`, `service` | + `default` | HTTP/1.1 connections with a header read timeout (not `axum::serve`); no `server-auto`, so no h2 |
| idna_adapter =1.1.0 | none | `compiled_data`, `default` | Pins lettre's IDNA backend to unicode-rs: 104 crates instead of 120 with the default 1.2 (no ICU4X: `icu_*`, `zerovec`, `yoke`, `tinystr`, … 21 crates) |
| lettre 0.11.23 | `smtp-transport`, `builder`, `tokio1-rustls`, `ring`, `webpki-roots` | + `rustls`, `tokio1` | SMTP over rustls (ring) with compiled Mozilla roots; no `pool`, no native TLS |
| rusqlite 0.40.2 | `bundled` | + `modern_sqlite` | SQLite compiled in (no system library) |
| serde 1.0.229 | `std`, `derive` | + `serde_derive` | JSON bodies |
| serde_json 1.0.151 | `std` | same | JSON bodies |
| sha2 0.11.0 | none | none | SHA-256 for HMAC, token hashes and asset names |
| tokio 1.53.1 | `rt-multi-thread`, `net`, `time`, `sync`, `signal`, `macros` | + `default` (from axum/hyper-util) | Runtime, listener, timers, semaphores, SIGTERM; no `io-util` |
| tower 0.5.3 | `util` | + `make`, `tokio` (from axum) | The dispatcher and header layer wrap the routers outside axum |
| tower-http 0.7.1 | `limit`, `timeout`, `trace` | + `tracing` | Body caps, the request deadline (408), request tracing |
| tracing 0.1.44 | `std` | same | Structured logs; no `attributes` proc-macro |
| tracing-subscriber 0.3.23 | `fmt`, `std` | + `registry` | Log output; no `env-filter`, so `RUST_LOG` changes nothing |

Dev-dependency: rustls 0.23.45 (`ring`, `std`, `tls12`), the version lettre uses, for the
local TLS peer of `tests/smtp_tls.rs`.

**Bundled SQLite:** 3.53.2 (`libsqlite3-sys` 0.38.2). Review it on every rusqlite bump.

Build scripts allowed (`deny.toml`): `getrandom`, `libc`, `httparse`, `serde`, `serde_core`,
`serde_json`, `zmij`, `unicode-joining-type` (target and feature detection, table
generation), `ring`, `rustls` (crypto assembly, feature detection), `proc-macro2`, `quote`
(proc-macro support), `libsqlite3-sys` (compiles the bundled SQLite with `cc`).

## Images, actions and release tools

| Pin | Version | Why |
|---|---|---|
| Base image | `gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3` | Runtime image: no shell, nonroot uid 65532 |
| Caddy | `caddy:2.11` (2.11.4 when pinned) | TLS in `docker-compose.yml` and the smoke test |
| Docker Engine | 27 or newer | `enable_ipv6` networks with fixed addresses (§14) |
| cosign | v3.1.3, via `sigstore/cosign-installer` v4.1.2 | Keyless signing, Sigstore bundle as OCI referrer |
| `actions/checkout` | v7.0.1 `3d3c42e5aac5ba805825da76410c181273ba90b1` | CI and release |
| `actions/upload-artifact` | v7.0.1 `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` | Binaries between jobs |
| `actions/download-artifact` | v8.0.1 `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` | Binaries between jobs |
| `docker/setup-buildx-action` | v4.4.1 `f87e5991a6d7451dcb8d9637bfbc97413f497069` | Multi-arch index with provenance |
| `sigstore/cosign-installer` | v4.1.2 `6f9f17788090df1f26f669e9d70d6ae9567deba6` | Installs cosign |
| actionlint | `rhysd/actionlint:1.7.12` (Docker) | Workflow lint, by hand |
