{
  description = "rusty-tip – tip preparation GUI & CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
    ...
  }: let
    forAllSystems = nixpkgs.lib.genAttrs ["x86_64-linux" "aarch64-linux"];
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
        version = "0.1.0";
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
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = ["--bin" "tip-prep"];
        nativeBuildInputs = [pkgs.pkg-config];
        doCheck = false;
      };

      tip-prep-gui-windows = pkgs.pkgsCross.mingwW64.rustPlatform.buildRustPackage {
        pname = "tip-prep-gui";
        version = "0.1.0";
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
      default = pkgs.mkShell {
        nativeBuildInputs =
          (with pkgs; [
            cargo
            clippy
            rustc
            rustfmt
            rust-analyzer
            pkg-config
            gcc
            cargo-expand
            cargo-dist
          ])
          ++ guiDeps;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiDeps;
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });
  };
}
