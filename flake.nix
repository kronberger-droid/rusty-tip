{
  description = "rusty-tip – tip preparation GUI & CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # The toolchain rides its own input so it is not welded to the nixpkgs pin:
    # `nix flake update rust-overlay` gets the newest stable without dragging
    # the whole package set forward. Same pattern as the NixOS config's
    # modules/home-manager/editors/dev-tools.nix.
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

    pkgsFor = system:
      import nixpkgs {
        inherit system;
        overlays = [rust-overlay.overlays.default];
      };

    # Hoisted: this list was previously spelled out identically in both the
    # package and the dev shell, so the two could silently disagree.
    guiDepsFor = pkgs:
      with pkgs; [
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
    packages = forAllSystems (system: let
      pkgs = pkgsFor system;
      guiDeps = guiDepsFor pkgs;
    in {
      # Deliberately nixpkgs' rustPlatform rather than one built on the overlay
      # toolchain: the packages should build the same way here and anywhere
      # that never fetched rust-overlay, and the mingwW64 cross below gets its
      # rustPlatform from pkgsCross, which the overlay does not reach into.
      # Toolchain currency is a dev-shell concern.
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
      pkgs = pkgsFor system;
      guiDeps = guiDepsFor pkgs;
      # `default` bundles rustc/cargo/clippy/rustfmt; rust-analyzer and rust-src
      # ride along as extensions, which is why RUST_SRC_PATH is gone:
      # rust-analyzer resolves the std sources through the toolchain's sysroot.
      toolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["rust-analyzer" "rust-src"];
      };
    in {
      default = pkgs.mkShell {
        nativeBuildInputs =
          [toolchain]
          ++ (with pkgs; [
            pkg-config
            gcc
            cargo-expand
            cargo-dist
          ])
          ++ guiDeps;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiDeps;

        shellHook = ''
          # Activate the repo's git hooks (pre-push rustfmt check).
          git config core.hooksPath .githooks 2>/dev/null || true
        '';
      };
    });
  };
}
