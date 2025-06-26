{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, fenix }:
    let eachSystem = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    in {
      devShells = eachSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ fenix.overlays.default ];
          };

          lib = pkgs.lib;
          toolchain = fenix.packages.${system}.stable.toolchain;
        in {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              pkg-config
              toolchain

              # test dependencies
              solc
              vyper
              dprint
              nodejs
            ];
            buildInputs = lib.optionals pkgs.stdenv.isDarwin
              [ pkgs.darwin.apple_sdk.frameworks.AppKit ];

            packages = with pkgs; [ rust-analyzer-unwrapped ];

            # Environment variables
            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
            LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.libusb1 ];
            CFLAGS = "-DJEMALLOC_STRERROR_R_RETURNS_CHAR_WITH_GNU_SOURCE";
          };
        });
    };
}
