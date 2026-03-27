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
  }:
    flake-utils.lib.eachDefaultSystem (system: let
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

      # NOTE: this thing comes from vmm repo that I was playing with, the disk creation and launching the vmm should work, the only thing to fix is the tap business or whatever
      diskVmLib = import ./lib/diskVm.nix {
        inherit pkgs nixpkgs system;
        extraPackages = [pkgs.curl pkgs.git pkgs.busybox pkgs.tmux pkgs.opencode];
        files = [
          {
            src = ../autonix;
            dst = "/opt/autonix";
          }
          {
            src = ./examples/python-workload/test.py;
            dst = "/usr/local/bin/test.py";
          }
          {
            src = ../docs;
            dst = "home";
          }
        ];
        sshKeys = [];
      };

      vmConfig = diskVmLib.vmConfig;

      rawImage = vmConfig.config.system.build.rawImage;
      qcow2Image = vmConfig.config.system.build.qcow2Image;

      # Build the full kernel cmdline: the user-specified params + init= pointing to toplevel
      kernelCmdline =
        builtins.concatStringsSep " " vmConfig.config.boot.kernelParams
        + " init=${vmConfig.config.system.build.toplevel}/init";

      # Helper script to launch the VM with cloud-hypervisor
      runVm = pkgs.writeShellScriptBin "run-vm" ''
        set -euo pipefail

        KERNEL="${vmConfig.config.boot.kernelPackages.kernel}/${vmConfig.config.system.boot.loader.kernelFile}"
        INITRD="${vmConfig.config.system.build.initialRamdisk}/initrd"
        STORE_DISK="${rawImage}/nixos.img"
        CMDLINE="${kernelCmdline}"

        DISK="''${1:-./nixos-vm.img}"

        echo "Copying disk image to writable location: $DISK"
        DISK="$(mktemp ./nixos-vm.XXXXXX.img)"
        cp --reflink=auto "$STORE_DISK" "$DISK"

        trap 'rm -f "$DISK"' EXIT


        echo "=== Cloud Hypervisor VM ==="
        echo "  Kernel:  $KERNEL"
        echo "  Initrd:  $INITRD"
        echo "  Disk:    $DISK"
        echo "  Cmdline: $CMDLINE"
        echo ""

        ${pkgs.cloud-hypervisor}/bin/cloud-hypervisor \
          --kernel "$KERNEL" \
          --initramfs "$INITRD" \
          --cmdline "$CMDLINE" \
          --disk path="$DISK" \
          --console off \
          --serial tty \
          --cpus boot=2 \
          --memory size=1024M \
          --net tap=tap0,mac=AA:00:00:00:00:01

        echo "=== Cloud Hypervisor VM stopped ==="
      '';
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

      packages =
        packageSet
        // {
          default = packageSet.worker;
          runVm = runVm;
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

          nativeBuildInputs = [pkgs.rustPackages.cargo pkgs.rustPackages.rustfmt pkgs.rustPackages.clippy];

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
