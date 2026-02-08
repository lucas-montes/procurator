{
  description = "A devShell example";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
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

        mkRustPackage = cargoDir: let
          cargoPath = ./${cargoDir}/Cargo.toml;
          cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
        in
          pkgs.rustPlatform.buildRustPackage {
            inherit pname version;
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            cargoBuildFlags = ["-p" pname];
            cargoInstallFlags = ["-p" pname];

            nativeBuildInputs = [pkgs.pkg-config pkgs.capnproto];
            buildInputs = [pkgs.openssl];
            doCheck = false;
          };

        cache = mkRustPackage "cache";

        ci_service = mkRustPackage "ci_service";
        repohub = mkRustPackage "repohub";

        procurator = mkRustPackage "procurator";

        cli = mkRustPackage "cli";

        pcr-dev = pkgs.writeShellScriptBin "pcr" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli -- "$@"
        '';

        pcr-test = pkgs.writeShellScriptBin "pcr-test" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli --bin pcr-test -- "$@"
        '';

      in {
        packages = {
          inherit cache ci_service procurator cli;
        };

        devShells.default = with pkgs;
          mkShell {
            buildInputs = [
              cargo-watch
              pkg-config
              cargo-watch
              rust-bin-custom
              capnproto

              pcr-dev
              pcr-test
            ];

            shellHook = ''
              export CARGO_MANIFEST_DIR="$PWD"
            '';
          };
      }
    );
}
