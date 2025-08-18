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

        # Dependencies for plotters (minimal)
        plottersDeps = with pkgs; [
          # Font rendering for plotters
          fontconfig
          freetype
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
        
        # Complete dependency set including plotters
        allDeps = rustDeps ++ plottersDeps;

        # Library path for plotters
        libPath = lib.makeLibraryPath plottersDeps;

        # Base shell configuration for plotters
        baseShellHook = ''
          echo "Using Rust toolchain: $(rustc --version)"
          export CARGO_HOME="$HOME/.cargo"
          export RUSTUP_HOME="$HOME/.rustup"
          
          # Library path for plotters font rendering
          export LD_LIBRARY_PATH="${libPath}:''${LD_LIBRARY_PATH:-}"
          
          # PKG_CONFIG setup for plotters native dependencies
          export PKG_CONFIG_PATH="${pkgs.fontconfig.dev}/lib/pkgconfig:${pkgs.freetype.dev}/lib/pkgconfig:''${PKG_CONFIG_PATH:-}"
          export PKG_CONFIG_ALLOW_SYSTEM_LIBS=1
          export PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
          
          mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"
          
          echo "Environment configured for plotters (fontconfig + freetype)"
        '';

        # Sway window manager setup for development layout
        swayDevSetup = ''
          # Dev setup - creates split terminal layout
          swaymsg layout splith
          
          swaymsg layout stacking
          
          swaymsg exec "kitty --working-directory=$(pwd) -e bash -c 'nix develop .#dev --command bash -c \"clear && nu --login\"'"
          sleep 0.5
          
          swaymsg focus parent
          
          swaymsg exec "kitty --working-directory=$(pwd) -e bash -c 'nix develop .#dev --command bash -c \"clear && claude\"'"
          sleep 0.5

          swaymsg layout stacking
          
          swaymsg focus left
          nix develop .#dev --command hx
        '';
      in {
        # Clean development shell without window manager setup
        devShells.dev = pkgs.mkShell {
          name = "rust dev shell (clean)";
          buildInputs = rustDeps ++ plottersDeps;
          shellHook = baseShellHook;
        };
        
        # Default shell with sway integration and window management
        devShells.default = pkgs.mkShell {
          name = "rust dev shell (with sway setup)";
          buildInputs = rustDeps;
          shellHook = baseShellHook + swayDevSetup;
        };

        # Minimal shell with just Rust toolchain (no GUI dependencies)
        devShells.minimal = pkgs.mkShell {
          name = "rust minimal shell";
          buildInputs = rustDeps;
          shellHook = baseShellHook;
        };

        # Comprehensive egui shell with full graphics support
        devShells.egui = pkgs.mkShell {
          name = "egui-comprehensive-shell";
          buildInputs = allDeps;
          shellHook = baseShellHook + ''
            echo "Comprehensive egui development environment"
            echo "Libraries loaded: wayland, x11, mesa, gl, egl"
            
            # Additional egui-specific debugging
            # export RUST_LOG=debug
            # export WINIT_UNIX_BACKEND=x11  # Uncomment to force X11
            
            # Launch nushell as login shell
            exec nu --login
          '';
        };
      }
    );
}
