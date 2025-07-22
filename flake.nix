{
  description = "Nanonis Rust library development shell";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url  = "github:numtide/flake-utils";
    fenix.url        = "github:nix-community/fenix";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, fenix, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default rust-overlay.overlays.default ];
        };
        lib = pkgs.lib;

        # Use Fenix's complete stable toolchain and latest rust-analyzer
        stableToolchain = fenix.packages.${system}.complete.toolchain;
        rustAnalyzer    = fenix.packages.${system}.latest.rust-analyzer;
      in {
        devShells.dev = pkgs.mkShell {
          name = "nanonis rust dev shell (bash)";

          buildInputs = with pkgs; lib.flatten [
            # rust tools
            stableToolchain
            rustAnalyzer
            cargo-expand
            rusty-man

            # build and runtime dependencies
          ];

          shellHook = ''
            echo "Using Rust toolchain: $(rustc --version)"
            export RUST_BACKTRACE=1
            # Ensure local cargo cache in home dir
            export CARGO_HOME="$HOME/.cargo"
            # Avoid accidental writes to Nix store; RUSTUP_HOME not used by Fenix
            export RUSTUP_HOME="$HOME/.rustup"
            mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"
          '';
        };
        
        devShells.default = pkgs.mkShell {
          name = "basic rust dev shell";

          buildInputs = with pkgs; lib.flatten [
            # rust tools
            stableToolchain
            rustAnalyzer
            cargo-expand
            rusty-man

            # build and runtime dependencies
          ];

          shellHook = ''
            echo "Using Rust toolchain: $(rustc --version)"
            export RUST_BACKTRACE=1
            # Ensure local cargo cache in home dir
            export CARGO_HOME="$HOME/.cargo"
            # Avoid accidental writes to Nix store; RUSTUP_HOME not used by Fenix
            export RUSTUP_HOME="$HOME/.rustup"
            mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"

            exec nu --login
          '';
        };
      }
    );
}
