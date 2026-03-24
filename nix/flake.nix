{
  description = "Procurator an orchestrator framework for your cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
  };

  outputs = {
    nixpkgs,
    rust-overlay,
    flake-utils,
    naersk ? null,
    ...
  }: flake-utils.lib.eachDefaultSystem (system: let
    overlays = [(import rust-overlay)];
    pkgs = import nixpkgs {
      inherit system overlays;
    };

    workspaceRoot = pkgs.lib.cleanSourceWith {
      src = ../.;
      filter = path: _type: let
        root = toString ../.;
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
      extensions = ["rust-src"];
    };

    packageSet = import ./flake/packages.nix {
      inherit pkgs workspaceRoot naersk;
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
      sandbox = import ./lib/sandbox/sandbox-module.nix;
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

    checks = {
      rust-lints = pkgs.stdenv.mkDerivation {
        name = "procurator-rust-lints";
        src = workspaceRoot;

        nativeBuildInputs = [ pkgs.rustPackages.cargo pkgs.rustPackages.rustfmt pkgs.rustPackages.clippy ];

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
  });
}
