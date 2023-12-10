{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    solc = {
      url = "github:hellwolf/solc.nix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, solc }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default solc.overlay ];
        };
        lib = pkgs.lib;
        toolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
          extensions = [ "rustfmt" "clippy" "rust-src" ];
        });
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            libusb1
          ] ++ lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.AppKit
          ];
          buildInputs = [
            pkgs.rust-analyzer-unwrapped
            toolchain
          ];
          packages = with pkgs; [
            solc_0_8_20
            (solc.mkDefault pkgs solc_0_8_20)
          ];

          # Environment variables
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.libusb1 ];
        };
      });
}
