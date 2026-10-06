{
  description = "rusty-tip – tip preparation GUI & CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # The dev-shell toolchain rides its own input so it tracks the newest
    # stable, the same channel CI's `dtolnay/rust-toolchain@stable` installs.
    # Pinned to nixpkgs' rustc it sat at 1.94 while CI ran a newer clippy,
    # so a locally clean tree failed CI. `nix flake update rust-overlay`
    # catches up without dragging nixpkgs forward.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    ...
  }: let
    forAllSystems = nixpkgs.lib.genAttrs ["x86_64-linux" "aarch64-linux"];
    # Single source of truth, so the packages cannot drift from the crate.
    version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      guiDeps = with pkgs; [
        wayland
        wayland-protocols
        libxkbcommon
        libX11
        libXcursor
        libXrandr
        libXi
        libGL
        libGLU
        gtk3
        dbus
        dbus.lib
        zenity
      ];
    in {
      tip-prep-gui = pkgs.rustPlatform.buildRustPackage {
        pname = "tip-prep-gui";
        inherit version;
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        buildFeatures = ["gui"];
        cargoBuildFlags = ["--bin" "tip-prep-gui"];
        nativeBuildInputs = [pkgs.pkg-config];
        buildInputs = guiDeps;
        doCheck = false;
      };

      tip-prep = pkgs.rustPlatform.buildRustPackage {
        pname = "tip-prep";
        inherit version;
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = ["--bin" "tip-prep"];
        nativeBuildInputs = [pkgs.pkg-config];
        doCheck = false;
      };

      tip-prep-gui-windows = pkgs.pkgsCross.mingwW64.rustPlatform.buildRustPackage {
        pname = "tip-prep-gui";
        inherit version;
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        buildFeatures = ["gui"];
        cargoBuildFlags = ["--bin" "tip-prep-gui"];
        doCheck = false;
        buildInputs = [];
      };

      default = self.packages.${system}.tip-prep-gui;
    });

    devShells = forAllSystems (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [rust-overlay.overlays.default];
      };
      # rust-analyzer finds std through the toolchain's sysroot via rust-src,
      # so RUST_SRC_PATH is no longer needed.
      toolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["rust-analyzer" "rust-src"];
      };
      guiDeps = with pkgs; [
        wayland
        wayland-protocols
        libxkbcommon
        libX11
        libXcursor
        libXrandr
        libXi
        libGL
        libGLU
        gtk3
        dbus
        dbus.lib
        zenity
      ];
    in rec {
      default = pkgs.mkShell {
        nativeBuildInputs =
          [toolchain]
          ++ (with pkgs; [
            pkg-config
            typos
            gcc
            cargo-expand
            cargo-dist
          ])
          ++ guiDeps;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiDeps;

        shellHook = ''
          # Activate the repo's git hooks (pre-push mirrors the CI gate).
          git config core.hooksPath .githooks 2>/dev/null || true
        '';
      };

      # The workbench in a headless sway, driven over VNC, for screenshots
      # without a desktop: `nix develop .#gui-test -c dev/headless-gui/gui.sh`.
      # Mesa comes from this nixpkgs rather than the system: the system Mesa
      # can need a newer glibc than the dev shell links, and then no EGL
      # config loads.
      gui-test = pkgs.mkShell {
        inputsFrom = [default];
        packages = with pkgs; [sway wayvnc grim];
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiDeps;
        HEADLESS_GUI_MESA = "${pkgs.mesa}";
      };

      # Python for dev/nanonis: Nanonis .dat/.sxm files and raw TCP calls.
      analysis = pkgs.mkShell {
        packages = [(pkgs.python3.withPackages (ps: [ps.numpy ps.scipy]))];
      };
    });
  };
}
