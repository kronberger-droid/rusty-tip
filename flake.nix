{
  description = "Rust development shell with GUI support";

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

        # Rust toolchain configuration
        rustTools = {
          stable = fenix.packages.${system}.stable.toolchain;
          analyzer = fenix.packages.${system}.latest.rust-analyzer;
        };

        # GUI dependencies for egui (from egui-test)
        guiDeps = with pkgs; [
          wayland
          wayland-protocols
          libxkbcommon
          libGL
        ];

        # Development tools
        devTools = with pkgs; [
          cargo-expand
          rusty-man
          nushell
          pkg-config
        ];

        # Core Rust development dependencies
        rustDeps = [
          rustTools.stable
          rustTools.analyzer
        ] ++ devTools;
        
        # Complete dependency set including GUI
        allDeps = rustDeps ++ guiDeps;

        # Library path for GUI
        libPath = lib.makeLibraryPath guiDeps;

        # Base shell configuration
        baseShellHook = ''
          echo "Using Rust toolchain: $(rustc --version)"
          export CARGO_HOME="$HOME/.cargo"
          export RUSTUP_HOME="$HOME/.rustup"
          
          # Library path for GUI
          export LD_LIBRARY_PATH="${libPath}:''${LD_LIBRARY_PATH:-}"
          
          mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"
          
          # Add dev command alias for easy re-entry
          alias dev="nix develop .#dev --command nu --login"
        '';

        # Create a startup script for better session management
        startupScript = pkgs.writeShellScript "dev-startup" ''
          #!/usr/bin/env bash
          set -e
          
          # Function to run app and return to nushell on exit
          run_with_fallback() {
            local app="$1"
            echo "Starting $app..."
            if ! "$app"; then
              echo "$app exited, starting nushell..."
            fi
            exec nu --login
          }
          
          # Check what to run based on argument
          case "''${1:-shell}" in
            hx|helix)
              run_with_fallback hx
              ;;
            claude)
              run_with_fallback claude
              ;;
            shell|*)
              exec nu --login
              ;;
          esac
        '';

        # Sway window manager setup for development layout
        swayDevSetup = ''
          # Dev setup - creates split terminal layout
          swaymsg layout splith
          
          swaymsg layout stacking
          
          swaymsg exec "kitty --working-directory=$(pwd) -e nix develop .#dev --command ${startupScript} shell"
          sleep 0.5
          
          swaymsg focus parent
          
          swaymsg exec "kitty --working-directory=$(pwd) -e nix develop .#dev --command ${startupScript} claude"
          sleep 0.5

          swaymsg layout stacking
          
          swaymsg focus left
          # Main terminal with helix
          nix develop .#dev --command ${startupScript} hx
        '';
      in {
        # Clean development shell without window manager setup
        devShells.dev = pkgs.mkShell {
          name = "rust dev shell (clean)";
          buildInputs = allDeps;
          shellHook = baseShellHook;
        };
        
        # Default shell with sway integration and window management
        devShells.default = pkgs.mkShell {
          name = "rust dev shell (with sway setup)";
          buildInputs = allDeps;
          shellHook = baseShellHook + swayDevSetup;
        };
      }
    );
}
