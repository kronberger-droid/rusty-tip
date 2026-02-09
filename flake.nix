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

        # GUI dependencies for eframe/egui
        guiDeps = with pkgs; [
          # Wayland
          wayland
          wayland-protocols
          libxkbcommon

          # X11 fallback
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi

          # OpenGL
          libGL
          libGLU

          # GTK for file dialogs (rfd)
          gtk3

          # D-Bus for portal file dialogs
          dbus
          dbus.lib

          # Zenity fallback for file dialogs
          zenity
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

        # GUI shell hook with library paths
        guiShellHook = ''
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath guiDeps}:$LD_LIBRARY_PATH"
        '';
      in {
        # Package builds
        packages = {
          # Linux GUI build
          tip-prep-gui = pkgs.rustPlatform.buildRustPackage {
            pname = "tip-prep-gui";
            version = "0.0.3";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            buildFeatures = ["gui"];
            buildAndTestSubdir = ".";
            cargoBuildFlags = ["--bin" "tip-prep-gui"];

            nativeBuildInputs = with pkgs; [pkg-config];
            buildInputs = guiDeps;

            # Skip tests during build
            doCheck = false;
          };

          # Linux CLI build (no GUI)
          tip-prep = pkgs.rustPlatform.buildRustPackage {
            pname = "tip-prep";
            version = "0.0.3";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            cargoBuildFlags = ["--bin" "tip-prep"];

            nativeBuildInputs = with pkgs; [pkg-config];

            doCheck = false;
          };

          # Windows GUI build (cross-compiled)
          tip-prep-gui-windows = pkgs.pkgsCross.mingwW64.rustPlatform.buildRustPackage {
            pname = "tip-prep-gui";
            version = "0.0.3";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            buildFeatures = ["gui"];
            cargoBuildFlags = ["--bin" "tip-prep-gui"];

            doCheck = false;

            # Windows doesn't need the Linux GUI deps
            buildInputs = [];
          };

          default = self.packages.${system}.tip-prep-gui;
        };

        # Clean development shell for Linux development (with GUI support)
        devShells.default = pkgs.mkShell {
          name = "rust dev shell (Linux + GUI)";
          buildInputs = rustDeps ++ guiDeps;
          shellHook = baseShellHook + guiShellHook;
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
