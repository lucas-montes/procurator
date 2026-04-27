{
  description = "Procurator an orchestrator framework for your cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      naersk ? null,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        workspaceRoot = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: _type:
            let
              root = toString ./.;
              pathStr = toString path;
              relPath = pkgs.lib.removePrefix "${root}/" pathStr;
            in
            !(
              pkgs.lib.hasPrefix ".git/" relPath
              || pkgs.lib.hasPrefix "target/" relPath
              || pkgs.lib.hasPrefix ".direnv/" relPath
              || pkgs.lib.hasPrefix "result/" relPath
              || relPath == "result"
              || pkgs.lib.hasPrefix "tmp/" relPath
            );
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
        };

        checks = {
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
        };
      }
    );
}
