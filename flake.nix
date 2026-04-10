{
  description = "rusty-tip – tip preparation GUI & CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {self, nixpkgs, fenix, ...}: let
    forAllSystems = nixpkgs.lib.genAttrs ["x86_64-linux" "aarch64-linux"];
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      guiDeps = with pkgs; [
        wayland wayland-protocols libxkbcommon
        xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi
        libGL libGLU gtk3 dbus dbus.lib zenity
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
      toolchain = fenix.packages.${system}.combine [
        (fenix.packages.${system}.stable.withComponents [
          "cargo" "clippy" "rust-src" "rustc" "rustfmt"
        ])
        fenix.packages.${system}.targets.x86_64-pc-windows-gnu.stable.rust-std
      ];
      guiDeps = with pkgs; [
        wayland wayland-protocols libxkbcommon
        xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi
        libGL libGLU gtk3 dbus dbus.lib zenity
      ];
    in {
      default = pkgs.mkShell {
        nativeBuildInputs = [
          toolchain
          fenix.packages.${system}.rust-analyzer
          pkgs.pkg-config
          pkgs.gcc
          pkgs.cargo-expand
          pkgs.cargo-dist
        ] ++ guiDeps;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath guiDeps;
      };

      windows = pkgs.mkShell {
        nativeBuildInputs = [
          toolchain
          fenix.packages.${system}.rust-analyzer
          pkgs.pkg-config
          pkgs.pkgsCross.mingwW64.stdenv.cc
          pkgs.pkgsCross.mingwW64.windows.pthreads
        ];
      };
    });
  };
}
