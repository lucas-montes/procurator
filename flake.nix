{
  description = "Procurator an orchestrator framework for your cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    sce.url = "github:crocoder-dev/shared-context-engineering";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      naersk,
      sce,
      ...
    }:
    {
      # Adds pkgs.cache, pkgs.ci_service, pkgs.worker, pkgs.cli,
      # pkgs.control_plane and pkgs.repohub to a consumer's pkgs set so the
      # NixOS modules' package defaults resolve out of the box.
      overlays.default = final: _: import ./nix/flake/overlay.nix {
        pkgs = final;
        src = self;
        inherit naersk;
      };
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        workspaceRoot = import ./nix/flake/workspaceRoot.nix {
          inherit pkgs;
          src = ./.;
        };

        rust-bin-custom = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };

        packageSet = import ./nix/flake/packages.nix {
          inherit pkgs workspaceRoot naersk;
        };

        workerLib = import ./nix/lib/worker.nix { inherit pkgs; };

        appSet = import ./nix/flake/apps.nix {
          inherit pkgs flake-utils workerLib;
          packages = packageSet;
        };
      in
      {
        nixosModules.procurator = import ./nix/modules;

        libs = import ./nix/lib {
          inherit pkgs nixpkgs system;
        };

        packages = packageSet // {
          default = packageSet.worker;
        };

        apps = appSet.apps;

        devShells.default = import ./nix/flake/shell.nix {
          inherit pkgs rust-bin-custom;
          sce = sce.packages.${system}.default;
        };

        checks =
          let
            # Pure-eval module tests (see nix/tests/*-fast.nix): force the
            # evaluation inside a check derivation, no builds, no VM.
            fastTest = name: file:
              pkgs.runCommand name {
                result = toString (
                  import file { lib = pkgs.lib; }
                );
              } "touch $out";
          in
          let
            # Pure NixOS eval: all five services composed into one real
            # system. Proves the modules wire their units/users/configs
            # under real option semantics (no build, no VM).
            services-system-eval =
              let
                overlaidPkgs = pkgs.extend (
                  final: _: import ./nix/flake/overlay.nix {
                    pkgs = final;
                    src = ./.;
                    inherit naersk;
                  }
                );
              in
              pkgs.runCommand "nixos-services-eval" {
                result = import ./nix/tests/services-system-eval.nix {
                  lib = nixpkgs.lib;
                  pkgs = overlaidPkgs;
                };
              } "touch $out";

            rust-lints = pkgs.stdenv.mkDerivation {
              name = "procurator-rust-lints";
              src = workspaceRoot;

              nativeBuildInputs = [
                pkgs.rustPackages.cargo
                pkgs.rustPackages.rustfmt
                pkgs.rustPackages.clippy
              ];

              buildPhase = ''
                cd "$src"
                cargo fmt --all -- --check
                cargo clippy --all-targets --all-features -- -D warnings
              '';

              installPhase = ''
                mkdir -p "$out"
                touch "$out"/.ok
              '';
            };
          in
          {
            inherit rust-lints services-system-eval;

            nixos-worker-fast = fastTest "nixos-worker-fast" ./nix/tests/worker-service-fast.nix;
            nixos-cache-fast = fastTest "nixos-cache-fast" ./nix/tests/cache-fast.nix;
            nixos-ci-service-fast = fastTest "nixos-ci-service-fast" ./nix/tests/ci-service-fast.nix;
            nixos-repohub-fast = fastTest "nixos-repohub-fast" ./nix/tests/repohub-fast.nix;
            nixos-control-plane-fast = fastTest "nixos-control-plane-fast" ./nix/tests/control-plane-fast.nix;
          };
      }
    );
}
