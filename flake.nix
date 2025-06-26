{
  description = "A devShell example";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust-bin-custom = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src"];
        };
      in {
        packages = {
          dummy = pkgs.rustPlatform.buildRustPackage {
            pname = "dummy";
            version = "0.1.0";

            src = ./dummy;

            cargoLock = {
              lockFile = ./dummy/Cargo.lock;
            };

            nativeBuildInputs = [
              rust-bin-custom
            ];

            meta = with pkgs.lib; {
              description = "A dummy executable for testing";
              license = licenses.mit;
            };
          };

          # Make dummy the default package
          default = self.packages.${system}.dummy;
        };

        devShells.default = with pkgs;
          mkShell {
            buildInputs = [
              pkg-config
              rust-bin-custom
            ];
          };
      }
    );
}
