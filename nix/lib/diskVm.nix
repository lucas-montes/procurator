{
  pkgs,
  nixpkgs,
  system ? builtins.currentSystem,
  extraPackages ? [pkgs.busybox],
  sshKeys ? [],
  files ? [],
  # Upstream DNS resolver for allowed domains.
  # Should point to the host bridge address so the VM goes through the host's dnsmasq.
  upstreamDns ? "192.168.100.1",
  # List of domains the VM is allowed to reach. All other DNS queries return 0.0.0.0.
  # Subdomains are included automatically, e.g. "github.com" also covers "api.github.com".
  # Example: [ "github.com" "pypi.org" ]
  allowedDomains ? [],
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
        resolvedFiles =
          map (
            f: {
              source = f.src;
              target = f.dst;
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
        boot = {
          loader = {
            grub.enable = false;
            # remove boot menu delay
            timeout = 0;
          };
          initrd.availableKernelModules = [
            "virtio_pci"
            "virtio_blk"
            "virtio_net"
            "virtio_console"
            "ext4"
          ];
          kernelParams = [
            "console=ttyS0"
            "root=/dev/vda"
            "rw"
            # fsck = "file system check" — it verifies and repairs filesystem inconsistencies at boot. Skipping it (fsck.mode=skip) makes boot faster but risks undetected corruption after an unclean shutdown.
            "fsck.mode=skip"
            "quiet"
          ];
        };

        # Filesystem – single root partition on /dev/vda
        fileSystems."/" = {
          device = "/dev/vda";
          fsType = "ext4";
          autoResize = true;
          # reduce unnecessary writes
          options = ["noatime"];
        };

        # Networking
        networking = {
          hostName = "cloud-vm";
          useDHCP = true;
          # speed up DHCP: don't block boot while dhcpcd waits for leases
          dhcpcd = {
            wait = "background";
            extraConfig = "noarp";
          };
        };
        # DNS filtering inside the VM:
        # - dnsmasq listens only on loopback (127.0.0.1), NOT on the network interface.
        #   The host bridge dnsmasq handles DHCP — this one only does DNS filtering.
        # - Allowed domains are forwarded to upstreamDns for real resolution.
        # - All other domains are blocked with 0.0.0.0 (connection refused).
        # - The allowed domain list is baked into the image from the allowedDomains argument.
        services = {
          dnsmasq = {
            enable = true;
            # Make the VM itself use dnsmasq for DNS (sets nameserver 127.0.0.1 in resolv.conf)
            resolveLocalQueries = true;
            settings = {
              # Only listen on loopback — do NOT serve DHCP or touch the network interface.
              # The host bridge dnsmasq already handles DHCP for this VM.
              listen-address = "127.0.0.1";
              bind-interfaces = true;
              no-dhcp-interface = "";

              # Forward each allowed domain to the upstream resolver.
              # Subdomains are covered automatically: "github.com" also matches "api.github.com".
              server = map (d: "/${d}/${upstreamDns}") allowedDomains;

              # Block everything not matched above — returns 0.0.0.0 (connection refused).
              address = "/#/0.0.0.0";
            };
          };
          openssh = {
            enable = true;
            settings.PermitRootLogin = "yes";
          };
          # disable logrotate (its timer caused delay); use the official service option
          logrotate = {
            enable = false;
          };
          # limit journal disk usage to avoid long journal maintenance stalls
          journald = {
            extraConfig = "SystemMaxUse=50m";
          };
        };

        # https://wiki.archlinux.org/title/Improving_performance#Storage_devices
        # https://majiehong.com/post/2021-07-30_slow_nixos_startup/
        # Users
        users = {
          users.root = {
            # lingering means the user session manager keeps running even when no user is logged in. For a VM where root just runs a workload, this is unnecessary
            linger = false;
            initialPassword = "nixos";
            openssh.authorizedKeys.keys = sshKeys;
          };
        };
        # Mask blocking wait-online / settle units that commonly delay boot.
        # Keep SSH enabled above; these masks prevent long network/device waits.
        systemd.services = {
          oomd.enable = true; #This one we might want to keep it disabled?
          NetworkManager-wait-online.enable = false;
          systemd-udev-settle.enable = false;
          # this is the systemd user session manager for root (uid 0). It enables per-user systemd services, timers, and socket activation under the root user. If you don't run any user-level systemd units for root,
          "user@0".enable = false;
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
