{
  description = "Procurator an orchestrator framework for your cluster";

  # Flake inputs: external dependencies pinned by flake.lock
  inputs = {
    # NixOS 25.11 stable channel - provides base system packages
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    # Rust toolchain overlay - provides rust-bin.stable for precise Rust versions
    rust-overlay.url = "github:oxalica/rust-overlay";
    # Multi-system helper - generates outputs for Linux, macOS, etc.
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
  # Generate outputs for all default systems (x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin)
    flake-utils.lib.eachDefaultSystem (
      system: let
        # Apply rust-overlay to nixpkgs to get rust-bin attribute
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        # Custom Rust toolchain with rust-src for IDE support (rust-analyzer)
        rust-bin-custom = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src"];
        };
        # Nixpkgs library for module evaluation and utility functions
        # Nixpkgs library for module evaluation and utility functions
        lib = nixpkgs.lib;

        # Helper function to build Rust packages from cargo workspaces
        # Takes a cargo directory name and builds that specific workspace member
        mkRustPackage = cargoDir: let
          # Read Cargo.toml to extract package name and version
          cargoPath = ../${cargoDir}/Cargo.toml;
          cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
        in
          pkgs.rustPlatform.buildRustPackage {
            inherit pname version;
            # Source is the entire workspace root (required for cargo workspace)
            src = ../.;

            # Use the workspace's Cargo.lock for reproducible builds
            cargoLock = {
              lockFile = ../Cargo.lock;
            };

            # Only build and install this specific workspace member
            cargoBuildFlags = ["-p" pname];
            cargoInstallFlags = ["-p" pname];

            # Build-time dependencies for compilation
            nativeBuildInputs = [pkgs.pkg-config pkgs.capnproto];
            # Runtime library dependencies
            buildInputs = [pkgs.openssl];
            # Skip tests during build (run separately if needed)
            doCheck = false;
          };

        # Build all Rust binaries from the workspace
        cache = mkRustPackage "cache"; # Binary cache service
        ci_service = mkRustPackage "ci_service"; # CI/CD pipeline service
        procurator = mkRustPackage "procurator"; # Main orchestrator binary
        cli = mkRustPackage "cli"; # Command-line interface

        # Wrapper scripts for running procurator in different modes
        # These allow the same binary to be invoked with different semantics
        procurator-worker = pkgs.writeShellScriptBin "procurator-worker" ''
          exec ${procurator}/bin/procurator "$@"
        '';

        procurator-control-plane = pkgs.writeShellScriptBin "procurator-control-plane" ''
          exec ${procurator}/bin/procurator "$@"
        '';

        # Development scripts using cargo run for faster iteration
        # These use the source code directly instead of building derivations
        pcr-dev = pkgs.writeShellScriptBin "pcr" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli -- "$@"
        '';

        pcr-test = pkgs.writeShellScriptBin "pcr-test" ''
          exec ${pkgs.cargo}/bin/cargo run --manifest-path="$CARGO_MANIFEST_DIR/procurator/Cargo.toml" -p cli --bin pcr-test -- "$@"
        '';
      in {
        # NixOS modules: importable configuration modules for NixOS systems
        # These define options and services that can be used in nixosConfigurations
        nixosModules = {
          # Cluster topology schema: defines VM configuration structure
          cluster = import ./modules/cluster.nix;
          # Service module for procurator worker nodes
          procurator-worker = import ./modules/procurator-worker.nix;
          # Service module for procurator control plane (master)
          procurator-control-plane = import ./modules/procurator-control-plane.nix;
          # Service module for binary cache server
          cache = import ./modules/cache.nix;
          # Service module for CI/CD pipeline
          ci-service = import ./modules/ci-service.nix;
          # Service module for git repository hosting
          repohub = import ./modules/repohub.nix;

          # Convenience module: imports all service modules at once
          # Use this to enable all procurator services in a single import
          default = {
            imports = [
              (import ./modules/procurator-worker.nix)
              (import ./modules/procurator-control-plane.nix)
              (import ./modules/cache.nix)
              (import ./modules/ci-service.nix)
              (import ./modules/repohub.nix)
            ];
          };
        };

        # Library functions: reusable Nix functions for cluster management
        lib.evalCluster = {clusterConfig}: let
          # Evaluate the cluster configuration using NixOS module system
          # This validates the config and provides error/warning messages
          eval = lib.evalModules {
            modules = [
              self.nixosModules.cluster
              clusterConfig
            ];
          };
        in {
          # The evaluated cluster configuration (VMs, deployment settings, etc.)
          config = eval.config.cluster;
          # Module system metadata: errors, warnings, and evaluation info
          # Check _module.warnings and _module.errors for validation results
          _module = eval._module;
        };

        # Packages: built derivations that can be installed or run
        # Access with: nix build '.#cache', nix build '.#procurator', etc.
        packages = {
          inherit cache ci_service procurator cli;
          # Default package when running 'nix build' without a specific target
          default = procurator;
        };

        # Apps: executable programs with nix run
        # These wrap packages to make them directly executable
        # Usage: nix run '.#cache', nix run '.#procurator-worker', etc.
        apps = {
          cache = flake-utils.lib.mkApp {drv = cache;};
          ci_service = flake-utils.lib.mkApp {drv = ci_service;};
          procurator = flake-utils.lib.mkApp {drv = procurator;};
          procurator-worker = flake-utils.lib.mkApp {drv = procurator-worker;};
          procurator-control-plane = flake-utils.lib.mkApp {drv = procurator-control-plane;};
          # Default app when running 'nix run' without a specific target
          default = flake-utils.lib.mkApp {drv = procurator-control-plane;};
        };

        # Development shell: environment for working on procurator
        # Enter with: nix develop
        devShells.default = with pkgs;
          mkShell {
            # Tools and dependencies available in the dev shell
            buildInputs = [
              cargo-watch # Auto-rebuild on file changes
              pkg-config # For linking system libraries
              rust-bin-custom # Rust toolchain with rust-analyzer support
              capnproto # Cap'n Proto schema compiler

              pcr-dev # Development CLI (uses cargo run)
              pcr-test # Test CLI (uses cargo run)

              openapi-generator-cli
            ];

            # Shell initialization script
            shellHook = ''
              export CARGO_MANIFEST_DIR="$PWD"
            '';
          };
      }
    );
}
