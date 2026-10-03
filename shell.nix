# Entorno de desarrollo para plugins de PORT en NixOS.

{ pkgs ? import <nixpkgs> { } }:

let
  inherit (pkgs) lib;
  runtimeLibs = with pkgs; [
    libxcb
    libxkbcommon
    freetype
    wayland
    vulkan-loader
  ];

  libDirs = lib.concatMapStringsSep ":" (package: "${package}/lib") runtimeLibs;

  rpathFlags = lib.concatMapStringsSep " " (package:
    "-C link-arg=-Wl,-rpath,${package}/lib") runtimeLibs;
in
pkgs.mkShell {
  name = "port-plugins-dev";

  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    pkg-config
  ];

  shellHook = ''
    echo "PORT PLUGINS: shell listo (fonte $(command -v cargo))"
    export LIBRARY_PATH="${libDirs}''${LIBRARY_PATH:+:$LIBRARY_PATH}"
    export RUSTFLAGS="${rpathFlags} ''${RUSTFLAGS:+ $RUSTFLAGS}"
  '';
}
