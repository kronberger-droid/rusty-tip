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
          config.allowUnfree = true;
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
          claude-code
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
        '';

        # Simple sway development layout setup
        swayDevSetup = ''
          # Create split terminal layout
          swaymsg layout splith
          swaymsg layout stacking
          
          # Open shell terminal
          swaymsg exec "kitty --working-directory=$(pwd) -e nix develop .#default"
          sleep 0.5
          
          swaymsg focus parent
          
          # Open claude terminal  
          swaymsg exec "kitty --working-directory=$(pwd) -e nix develop .#default -c nu --login -c ${pkgs.claude-code}/bin/claude"
          sleep 0.5

          swaymsg layout stacking
          swaymsg focus left
          
          # Start helix in main terminal
          echo "Starting helix..."
          hx .

          # Switch to nushell after helix exits
          exec nu --login
        '';
      in {
        # Clean development shell without window manager setup
        devShells.default= pkgs.mkShell {
          name = "rust dev shell (clean)";
          buildInputs = allDeps;
          shellHook = baseShellHook + ''
            exec nu --login
          '';
        };
        
        # Default shell with sway integration and window management
        devShells.dev= pkgs.mkShell {
          name = "rust dev shell (with sway setup)";
          buildInputs = allDeps;
          shellHook = baseShellHook + swayDevSetup;
        };
      }
    );
}
