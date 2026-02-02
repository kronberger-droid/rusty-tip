{
  description = "Rust development shell with GUI support";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix.url = "github:nix-community/fenix";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    fenix,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [fenix.overlays.default rust-overlay.overlays.default];
        };

        rustTools = {
          stable = pkgs.rust-bin.stable."1.89.0".default.override {
            extensions = ["rust-src"];
            targets = ["x86_64-pc-windows-gnu"];
          };
          analyzer = pkgs.rust-bin.stable."1.89.0".rust-analyzer;
        };

        # Development tools
        devTools = with pkgs; [
          cargo-expand
          pkg-config
          gcc
        ];

        # Windows cross-compilation tools
        windowsTools = with pkgs; [
          pkgsCross.mingwW64.stdenv.cc
          pkgsCross.mingwW64.windows.pthreads
        ];

        # Core Rust development dependencies
        rustDeps =
          [
            rustTools.stable
            rustTools.analyzer
          ]
          ++ devTools;

        # Base shell configuration
        baseShellHook = ''
          echo "Using Rust toolchain: $(rustc --version)"
          export CARGO_HOME="$HOME/.cargo"
          export RUSTUP_HOME="$HOME/.rustup"


          mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"
        '';
      in {
        # Clean development shell for Linux development
        devShells.default = pkgs.mkShell {
          name = "rust dev shell (Linux)";
          buildInputs = rustDeps;
          shellHook = baseShellHook;
        };

        # Windows cross-compilation shell
        devShells.windows = pkgs.mkShell {
          name = "rust dev shell (Windows cross-compile)";
          buildInputs = rustDeps ++ windowsTools;
          shellHook =
            baseShellHook
            + ''
              echo "Windows cross-compilation enabled"
              echo "Build with: cargo build --target x86_64-pc-windows-gnu"
            '';
        };
      }
    );
}
