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

            nativeBuildInputs = [pkgs.pkg-config];
            buildInputs = [pkgs.openssl];
            doCheck = false;
          };

        cache = mkRustPackage "cache";

        ci_service = mkRustPackage "ci_service";

        worker = mkRustPackage "worker";
        control_plane = mkRustPackage "control_plane";

        procfile = pkgs.writeTextFile {
          name = "Procfile";
          text = ''
            cache: ${cache}/bin/cache
            ci_service: ${ci_service}/bin/ci_service
          '';
        };

        github = pkgs.writeShellScriptBin "github" ''
          cd $(mktemp -d)
          cp ${procfile} Procfile
          ${pkgs.overmind}/bin/overmind start
        '';
      in {
        packages = {
          inherit cache worker ci_service control_plane github;
        };

        devShells.default = with pkgs;
          mkShell {
            buildInputs = [
              pkg-config
              cargo-watch
              rust-bin-custom
              capnproto
            ];
          };
      }
    );
}
