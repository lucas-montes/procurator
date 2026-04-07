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

    networkTest = pkgs.writeShellScript "network-test" ''
      #!/usr/bin/env bash
      set -euo pipefail

      echo "=== Network Test ==="
      echo "Testing connectivity to github.com..."
      curl -I https://github.com
      ping -c 4 github.com
      echo "Testing connectivity to pypi.org..."
      curl -I https://pypi.org
      ping -c 4 pypi.org
      echo "Testing connectivity to google.com..."
      curl -I https://google.com
      ping -c 4 google.com
      echo "All tests passed!"
    '';
    vm = pLibs.diskVm {
      extraPackages = [pkgs.curl pkgs.git pkgs.python3 pkgs.busybox pkgs.tmux pkgs.opencode];
      files = [
        {
          src = ../../../autonix;
          dst = "/opt/autonix";
        }
        {
          src = networkTest;
          dst = "/root/netest.sh";
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
          dst = "/root/test.py";
        }
      ];
      sshKeys = ["ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC4duRrJKDgzq3FXyHyfYVqqqLjTJlZAPmbCQOV236u1VBJyZMGpD+u+PJ4HSxIA/CFKOVIRcCahIGiW1L+7c33K7rmBuGGuE+hzzlkG7uR/Se0kcPMiLTtCf9mTVFeIruXWcI8CptICofjKLYaNw15BewsqQiuHHmiGffk4f5/4w8eWmSns+VE3H4r/4BYUsdJI4Kk+EpVG1Dz9Gf8YcbQCB0YxPycL3Cg4KApaMDkYDHlDNzOsB943IKKPDqzGIxXBLzqqndFlz3OgMbz7bbiiZckWnR6XDctED1UP9EhoEYb1CrwhMc2ldIcnvD0kVy1EytwXZ29MdBQiC8hDmgnNsVXEO9L2rfwybDVhN9owG9oHGb8X/LwUsrYxOqaMe1saR7v4BH5PzY4SW1kWWbh3wRsr/CjoWBZBPPE7Ln9QeN+VutbrkgpYWDsDtaxFHl2TvjdYwtmA21i1QH70dJDRtH4KWmwUUwYinDGfWWjHtGOR6r3MkDs1aX5QMlR/9M= lucas@laptop"];
      allowedDomains = [ "github.com" "pypi.org" "google.com"];
    };

    vmConfig = vm.vmConfig;

    # Build the full kernel cmdline: the user-specified params + init= pointing to toplevel
    kernelCmdline =
      builtins.concatStringsSep " " vmConfig.config.boot.kernelParams
      + " init=${vmConfig.config.system.build.toplevel}/init";

    # Dev/test launcher. Creates tap0 (if missing), attaches it to br0, runs the VM, then cleans up.
    # Requires root (sudo nix run .#runVm). In production the worker manages TAPs dynamically.
    runVm = pkgs.writeShellScriptBin "run-vm" ''
      set -euo pipefail

      KERNEL="${vmConfig.config.boot.kernelPackages.kernel}/${vmConfig.config.system.boot.loader.kernelFile}"
      INITRD="${vmConfig.config.system.build.initialRamdisk}/initrd"
      STORE_DISK="${vmConfig.config.system.build.rawImage}/nixos.img"
      CMDLINE="${kernelCmdline}"
      TAP="tap0"
      BRIDGE="br0"

      # Set up TAP device and attach to bridge if not already done.
      # Requires root (script is typically run with sudo).
      if ip link show "$TAP" &>/dev/null; then
        ip link delete "$TAP" 2>/dev/null || true
      fi
      echo "Creating TAP device $TAP and attaching to $BRIDGE..."
      ip tuntap add dev "$TAP" mode tap
      ip link set "$TAP" master "$BRIDGE"
      ip link set "$TAP" up

      DISK="$(mktemp ./nixos-vm.XXXXXX.img)"
      echo "Copying disk image to writable location: $DISK"
      cp --reflink=auto "$STORE_DISK" "$DISK"

      # Clean up disk and TAP on exit.
      cleanup() {
        rm -f "$DISK"
        ip link delete "$TAP" 2>/dev/null || true
      }
      trap cleanup EXIT

      echo "=== Cloud Hypervisor VM ==="
      echo "  Kernel:  $KERNEL"
      echo "  Initrd:  $INITRD"
      echo "  Disk:    $DISK"
      echo "  Cmdline: $CMDLINE"
      echo "  TAP:     $TAP -> $BRIDGE"
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
        --net tap="$TAP"

      echo "=== Cloud Hypervisor VM stopped ==="
    '';

  in {
    packages.${system} = {
      inherit runVm;
    };

    apps.${system} = {
      runVm = {
        type = "app";
        program = "${runVm}/bin/run-vm";
      };
    };
  };
}
