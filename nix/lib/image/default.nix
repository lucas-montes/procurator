# mkVmImage — Build a NixOS ext4 raw disk image for Cloud Hypervisor.
#
# Takes a VM profile (from mkVmProfile) and image-build options.
# Produces: { image, nixos, toplevel, vmSpec, vmSpecJson }
#
# Usage:
#   mkVmImage = import ./image { inherit pkgs nixpkgs system; };
#   vm = mkVmImage {
#     profile = mkVmProfile { hostname = "sandbox"; cpu = 2; memoryMb = 1024; };
#   };
#   # vm.image      — ext4 disk image derivation
#   # vm.vmSpec     — attrset matching 8-field capnp VmSpec
#   # vm.vmSpecJson — derivation producing vm-spec.json
{
  pkgs,
  nixpkgs,
  system,
}:
# mkVmImage function
{
  # Required: a validated profile from mkVmProfile
  profile,
  # Image-build options (NOT in the profile — these are artifact concerns)
  kernel ? pkgs.linux_6_6,
  diskSize ? "auto",
  additionalSpace ? "256M",
}: let
  lib = pkgs.lib;

  # ── Validate profile ────────────────────────────────────────────────
  _ = assert (profile._type or null)
  == "vmProfile"
  || builtins.throw "mkVmImage: 'profile' must be a validated vmProfile (from mkVmProfile)"; true;

  # ── Build the NixOS system ──────────────────────────────────────────
  nixos = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      # Base VM module (systemd, SSH, serial, virtio, minimal)
      ./vm-module.nix

      # Use the specified kernel
      {boot.kernelPackages = pkgs.linuxPackagesFor kernel;}

      # Apply profile settings to the guest
      ({pkgs, ...}: {
        ch-vm = {
          hostname = profile.hostname;
          sshAuthorizedKeys = profile.sshAuthorizedKeys;
          entrypoint = profile.entrypoint;
          autoShutdown = profile.autoShutdown;
          files = profile.files;
          allowedDomains = profile.allowedDomains;
          extraPackages = profile.packages pkgs;
        };
      })

      # ── First-boot setup (sd-image pattern) ──────────────────────
      # Register the Nix store and create the system profile on first boot.
      ({
        config,
        pkgs,
        ...
      }: {
        boot.postBootCommands = ''
          if [ -f /nix-path-registration ]; then
            set -euo pipefail
            ${pkgs.nix}/bin/nix-store --load-db < /nix-path-registration
            touch /etc/NIXOS
            ${pkgs.nix}/bin/nix-env -p /nix/var/nix/profiles/system --set /run/current-system
            rm -f /nix-path-registration
          fi
        '';
      })
    ];
  };

  toplevel = nixos.config.system.build.toplevel;

  # ── Build the disk image ────────────────────────────────────────────
  # Uses make-ext4-fs.nix (sd-image pattern — NO QEMU at build time).
  image = pkgs.callPackage "${nixpkgs}/nixos/lib/make-ext4-fs.nix" {
    compressImage = false;
    volumeLabel = "nixos";
    inherit diskSize additionalSpace;
    storePaths = [toplevel];
    populateImageCommands = ''
      # Minimal filesystem skeleton
      mkdir -p ./files/sbin
      ln -s ${toplevel}/init ./files/sbin/init

      mkdir -p ./files/etc
      touch ./files/etc/NIXOS

      mkdir -p ./files/var/log
      mkdir -p ./files/var/lib
      mkdir -p ./files/proc
      mkdir -p ./files/sys
      mkdir -p ./files/dev
      mkdir -p ./files/tmp
      mkdir -p ./files/run
      mkdir -p ./files/root

      mkdir -p ./files/nix/var/nix/profiles/per-user/root
      mkdir -p ./files/nix/var/nix/db
      mkdir -p ./files/nix/var/nix/gcroots

      ln -s ${toplevel} ./files/nix/var/nix/profiles/system
      ln -s ${toplevel}/etc/os-release ./files/etc/os-release

      # User-injected files (profile.files)
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList (path: content: ''
            mkdir -p ./files$(dirname ${path})
            cat > ./files${path} <<'__CH_VM_EOF__'
          ${content}
          __CH_VM_EOF__
        '')
        profile.files)}
    '';
  };

  # ── VM spec ─────────────────────────────────────────────────────────
  # Matches the 8-field capnp VmSpec schema exactly.
  vmSpec = {
    toplevel = toString toplevel;
    kernelPath = "${nixos.config.system.build.kernel}/${nixos.config.system.boot.loader.kernelFile}";
    initrdPath = "${nixos.config.system.build.initialRamdisk}/${nixos.config.system.boot.loader.initrdFile}";
    diskImagePath = toString image;
    cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
    cpu = profile.cpu;
    memoryMb = profile.memoryMb;
    networkAllowedDomains = profile.allowedDomains;
  };

  vmSpecJson = pkgs.writeText "vm-spec.json" (builtins.toJSON vmSpec);
in {
  inherit image nixos toplevel vmSpec vmSpecJson;
}
