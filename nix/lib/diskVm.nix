{
  pkgs,
  nixpkgs,
  system ? builtins.currentSystem,
  extraPackages ? [pkgs.busybox],
  sshKeys ? [],
  files ? [],
  ...
}: {
  vmConfig = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      ({
        config,
        pkgs,
        lib,
        modulesPath,
        ...
      }: let
        # Resolve destination paths
        # The format for content is {source, target, mode, user, group}
        resolvedFiles = map (
          f: let
            dst =
              if f.dst == "home"
              then config.users.users.root.home
              else f.dst;
          in {
            source = f.src;
            target = dst;
          }
        )
        files;

        buildSystem = format:
          import "${nixpkgs}/nixos/lib/make-disk-image.nix" {
            inherit pkgs lib config format;
            contents = resolvedFiles;
            diskSize = "auto";
            additionalSpace = "512M";
            partitionTableType = "none";
            installBootLoader = false;
            copyChannel = false;
          };
      in {
        imports = ["${modulesPath}/profiles/qemu-guest.nix"];

        # Boot configuration for Cloud Hypervisor (direct kernel boot)
        boot.loader.grub.enable = false;
        boot.initrd.availableKernelModules = [
          "virtio_pci"
          "virtio_blk"
          "virtio_net"
          "virtio_console"
          "ext4"
        ];
        boot.kernelParams = [
          "console=ttyS0"
          "root=/dev/vda"
          "rw"
        ];

        # Filesystem – single root partition on /dev/vda
        fileSystems."/" = {
          device = "/dev/vda";
          fsType = "ext4";
          autoResize = true;
        };

        # Networking
        networking = {
          hostName = "cloud-vm";
          useDHCP = true;
        };

        # Services
        services = {
          openssh = {
            enable = true;
            settings.PermitRootLogin = "yes";
          };
        };

        # Users
        users = {
          users.root = {
            initialPassword = "nixos";
            openssh.authorizedKeys.keys = sshKeys;
          };
        };

        # Extra packages
        environment.systemPackages = extraPackages;

        system = {
          stateVersion = "25.11";
          build = {
            # Build disk images
            rawImage = buildSystem "raw";
            qcow2Image = buildSystem "qcow2";
          };
        };
      })
    ];
  };
}
