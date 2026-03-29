{
  description = "Python workload example for procurator worker";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    procurator.url = "path:../..";
  };

  outputs = {
    self,
    nixpkgs,
    procurator,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};
    pLibs = procurator.libs.${system};

    vm = pLibs.diskVm {
      extraPackages = [pkgs.curl pkgs.git pkgs.python3 pkgs.busybox pkgs.tmux pkgs.opencode];
      files = [
        {
          src = ../../../autonix;
          dst = "/opt/autonix";
        }
        # Fix the issue where I need to specify the file when copying a file to a dir.
        #  In this case, pointing the docs to the home dir it unpacks the dire in home
        # {
        #   src = ../../../docs;
        #   dst = "home";
        # }
        # But then, when I want to copy test.py an error ocurs. Also copying test.py to /usr/local/bin converts bin into the file test.py
        {
          src = ./test.py;
          dst = "home/test.py";
        }
      ];
      sshKeys = [];
    };

    vmConfig = vm.vmConfig;

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

      # Sometimes, depending on the version of ch I need to specify image_type
      ${pkgs.cloud-hypervisor}/bin/cloud-hypervisor \
        --kernel "$KERNEL" \
        --initramfs "$INITRD" \
        --cmdline "$CMDLINE" \
        --disk path="$DISK",image_type="raw" \
        --console off \
        --serial tty \
        --cpus boot=2 \
        --memory size=1024M \
        --net tap=tap0,mac=AA:00:00:00:00:01

      echo "=== Cloud Hypervisor VM stopped ==="
    '';

    cli = procurator.packages.${system}.cli;
  in {
    packages.${system} = {
      inherit runVm;
    };

    apps.${system} = {
    };
  };
}
