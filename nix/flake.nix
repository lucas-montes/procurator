{
  description = "Procurator an orchestrator framework for your cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }: flake-utils.lib.eachDefaultSystem (system: let
    overlays = [(import rust-overlay)];
    pkgs = import nixpkgs {
      inherit system overlays;
    };

    workspaceRoot = builtins.path {
      path = ../.;
      name = "workspace-src";
    };

    rust-bin-custom = pkgs.rust-bin.stable.latest.default.override {
      extensions = ["rust-src"];
    };

    packageSet = import ./flake/packages.nix {
      inherit pkgs workspaceRoot;
    };

    appSet = import ./flake/apps.nix {
      inherit pkgs flake-utils;
      packages = packageSet;
    };
  in {
    nixosModules = {
      cluster = import ./modules/cluster.nix;
      host = import ./modules/host;
      procurator-worker = import ./modules/procurator-worker.nix;
      procurator-control-plane = import ./modules/procurator-control-plane.nix;
      cache = import ./modules/cache.nix;
      ci-service = import ./modules/ci-service.nix;
      repohub = import ./modules/repohub.nix;
      guest = import ./lib/image/vm-module.nix;
    };

    lib = import ./lib {
      inherit pkgs nixpkgs system;
    };

    packages = packageSet // {
      default = packageSet.worker;
    };

    apps = appSet.apps;

    devShells.default = import ./flake/shell.nix {
      inherit pkgs rust-bin-custom;
      pcr-test-wrapper = appSet.wrappers.pcr-test-wrapper;
    };
  });
}
