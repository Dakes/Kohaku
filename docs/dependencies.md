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
