{
  description = "Procurator an orchestrator framework for your cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
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
        lib = nixpkgs.lib;

        mkRustPackage = cargoDir: let
          cargoPath = ../${cargoDir}/Cargo.toml;
          cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
        in
          pkgs.rustPlatform.buildRustPackage {
            inherit pname version;
            src = ../.;

            cargoLock = {
              lockFile = ../Cargo.lock;
            };

            cargoBuildFlags = ["-p" pname];
            cargoInstallFlags = ["-p" pname];

            nativeBuildInputs = [pkgs.pkg-config pkgs.capnproto];
            buildInputs = [pkgs.openssl];
            doCheck = false;
          };

        cache = mkRustPackage "cache";
        ci_service = mkRustPackage "ci_service";
        procurator = mkRustPackage "procurator";
        cli = mkRustPackage "cli";

        procurator-worker = pkgs.writeShellScriptBin "procurator-worker" ''
          exec ${procurator}/bin/procurator "$@"
        '';

        procurator-control-plane = pkgs.writeShellScriptBin "procurator-control-plane" ''
          exec ${procurator}/bin/procurator "$@"
        '';

        pcr-dev = pkgs.writeShellScriptBin "pcr" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli -- "$@"
        '';

        pcr-test = pkgs.writeShellScriptBin "pcr-test" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli --bin pcr-test -- "$@"
        '';
      in {
        # Export NixOS modules for use in other flakes
        nixosModules = {
          cluster = import ./modules/cluster.nix;
          procurator-worker = import ./modules/procurator-worker.nix;
          procurator-control-plane = import ./modules/procurator-control-plane.nix;

          # Convenience: import both service modules at once
          default = { imports = [
            (import ./modules/procurator-worker.nix)
            (import ./modules/procurator-control-plane.nix)
          ]; };
        };

        # Expose a library function to evaluate a cluster config
        lib.evalCluster = { clusterConfig }:
          let
            eval = lib.evalModules {
              modules = [
                self.nixosModules.cluster
                clusterConfig
              ];
            };
          in {
            # Raw evaluated config (for control plane to read)
            config = eval.config.cluster;
            # Also expose errors/warnings if evaluation failed
            _module = eval._module;
          };

        packages = {
          inherit cache ci_service procurator cli;
          default = procurator;
        };

        apps = {
          cache = flake-utils.lib.mkApp { drv = cache; };
          ci_service = flake-utils.lib.mkApp { drv = ci_service; };
          procurator = flake-utils.lib.mkApp { drv = procurator; };
          procurator-worker = flake-utils.lib.mkApp { drv = procurator-worker; };
          procurator-control-plane = flake-utils.lib.mkApp { drv = procurator-control-plane; };
          default = flake-utils.lib.mkApp { drv = procurator-control-plane; };
        };

        devShells.default = with pkgs;
          mkShell {
            buildInputs = [
              cargo-watch
              pkg-config
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
