{
  pkgs,
  nixpkgs,
  system ? builtins.currentSystem,
  extraPackages ? [pkgs.busybox],
  sshKeys ? [],
  files ? [],
  # Upstream DNS resolver used by dnsmasq inside the VM to resolve allowed domains.
  # Should point to the host bridge address so the VM goes through the host's dnsmasq.
  upstreamDns ? "192.168.100.1",
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
        # - Allowed domains are forwarded to the host bridge (upstreamDns) for real resolution.
        # - All other domains are blocked with 0.0.0.0 (connection refused — no NXDOMAIN spoofing).
        # - The allowed domain list is injected at VM launch time via the kernel cmdline
        #   as `allowed_domains=domain1.com,domain2.com`. A one-shot systemd service reads
        #   /proc/cmdline at boot and writes the dnsmasq config before dnsmasq starts.
        services = {
          dnsmasq = {
            enable = true;
            # resolveLocalQueries: make the VM itself use dnsmasq for DNS (via 127.0.0.1)
            resolveLocalQueries = true;
            settings = {
              # Only listen on loopback — do NOT touch the network interface or serve DHCP.
              # The host bridge dnsmasq already handles DHCP for this VM.
              listen-address = "127.0.0.1";
              bind-interfaces = true;
              no-dhcp-interface = "";

              # The allowed domain list is written at runtime by dns-filter-setup.service
              # into /run/dnsmasq-allowed.conf before dnsmasq starts.
              conf-file = "/run/dnsmasq-allowed.conf";

              # Block everything that isn't matched by the per-domain server= lines.
              # Returns 0.0.0.0 (connection refused) for blocked domains.
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

          oomd = {
            enable = true;
            enableRootSlice = false; # don't kill system services
            enableUserSlices = false; # don't kill user sessions
            enableSystemSlice = false;
          };
        };

        # dns-filter-setup.service: reads allowed_domains= from /proc/cmdline and
        # writes /run/dnsmasq-allowed.conf before dnsmasq starts.
        # The kernel cmdline is set by the Rust worker at VM launch time.
        # Format: allowed_domains=github.com,pypi.org,example.com
        systemd.services.dns-filter-setup = {
          description = "Write dnsmasq allowed-domain config from kernel cmdline";
          wantedBy = [ "dnsmasq.service" ];
          before = [ "dnsmasq.service" ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
          };
          script = ''
            # Read allowed_domains=... from kernel cmdline
            CMDLINE=$(cat /proc/cmdline)
            DOMAINS=""
            for param in $CMDLINE; do
              case "$param" in
                allowed_domains=*)
                  DOMAINS="''${param#allowed_domains=}"
                  ;;
              esac
            done

            CONF=/run/dnsmasq-allowed.conf
            : > "$CONF"

            if [ -z "$DOMAINS" ]; then
              echo "# No allowed_domains= found in kernel cmdline — all DNS blocked" >> "$CONF"
            else
              # Each comma-separated domain gets a server= line forwarding to the upstream.
              # Subdomains are included automatically by dnsmasq (server=/github.com/x.x.x.x
              # matches api.github.com, raw.githubusercontent.com, etc.).
              IFS=',' read -r -a DOMAIN_LIST <<< "$DOMAINS"
              for domain in "''${DOMAIN_LIST[@]}"; do
                echo "server=/$domain/${upstreamDns}" >> "$CONF"
              done
            fi
          '';
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
