{
  description = "Kohaku, a tiny self-hosted bug report inbox";

  inputs = {
    # flake.lock pins a revision whose `openspec` is 1.13.1, the version AGENTS.md names.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          let
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            };
          in
          f {
            inherit pkgs;
            # The one toolchain pin, shared with rustup; never nixpkgs' rustc.
            toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          }
        );
    in
    {
      packages = forAllSystems (
        { pkgs, toolchain }:
        let
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          fs = pkgs.lib.fileset;
        in
        {
          # Host-platform build with the test suite; release binaries come from CI.
          default = rustPlatform.buildRustPackage {
            pname = "kohaku";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = fs.toSource {
              root = ./.;
              fileset = fs.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            doCheck = true;
            meta = {
              description = "A tiny, self-hosted bug report inbox";
              homepage = "https://github.com/Dakes/Kohaku";
              license = pkgs.lib.licenses.agpl3Plus;
              mainProgram = "kohaku";
            };
          };
        }
      );

      devShells = forAllSystems (
        { pkgs, toolchain }:
        {
          default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.just
              pkgs.watchexec
              pkgs.cargo-deny
              pkgs.cargo-zigbuild
              pkgs.zig
              pkgs.sqlite
              pkgs.openspec
              pkgs.file
            ];
          };
        }
      );
    };
}
