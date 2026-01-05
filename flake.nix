{
  description = "Alacritty devshell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        lib = pkgs.lib;

        rustToolchain = pkgs.rust-bin.stable."1.85.0".default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
        };

        darwinFrameworks = lib.optionals pkgs.stdenv.isDarwin (with pkgs.darwin.apple_sdk.frameworks; [
          AppKit
          Cocoa
          CoreGraphics
          CoreText
          CoreVideo
          IOKit
          Metal
          OpenGL
          QuartzCore
        ]);

        darwinLibraries = lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];

        linuxLibraries = lib.optionals pkgs.stdenv.isLinux (with pkgs; [
          fontconfig
          freetype
          libxkbcommon
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libxcb
          xorg.xcbutil
          xorg.xcbutilwm
        ]);
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustToolchain
            cmake
            pkg-config
            python3
            gnumake
            (writeShellScriptBin "run" ''
              exec cargo run -p alacritty --bin alacritty
            '')
          ] ++ darwinFrameworks ++ darwinLibraries ++ linuxLibraries;

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "Alacritty dev shell activated."
          '';
        };
      });
}
