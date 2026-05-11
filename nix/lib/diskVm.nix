{
  pkgs,
  nixpkgs,
  system ? builtins.currentSystem,
  extraPackages ? [pkgs.busybox],
  sshKeys ? [],
  files ? [],
  # Upstream DNS resolver for allowed domains.
  # Should point to the host bridge address so the VM goes through the host's dnsmasq.
  upstreamDns,
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

            "net.ifnames=0" # add this
            "biosdevname=0" # and this
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

        networking = {
          hostName = "cloud-vm";
          useDHCP = false;
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
            alwaysKeepRunning = true;
            # Make the VM itself use dnsmasq for DNS (sets nameserver 127.0.0.1 in resolv.conf)
            resolveLocalQueries = true;
            settings = {
              # Only listen on loopback — do NOT serve DHCP or touch the network interface.
              # The host bridge dnsmasq already handles DHCP for this VM.
              listen-address = "127.0.0.1";
              bind-interfaces = true;

              # Forward each allowed domain to the upstream resolver.
              # Subdomains are covered automatically: "github.com" also matches "api.github.com".
              server = map (d: "/${d}/${upstreamDns}") allowedDomains;

              # Block everything not matched above — returns 0.0.0.0 (connection refused).
              # address = "/#/0.0.0.0";

              # Everything else: NXDOMAIN for both A and AAAA.
              # `local=/#/` means "this domain is local; no upstream";
              # combined with no local records it returns NXDOMAIN.
              # Equivalent shorthand: server = [ "/#/" ] AFTER the allow list,
              # but `local` is clearer about intent.
              local = ["/#/"];
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

          # Parse procurator.* tokens from /proc/cmdline and apply them to eth0.
          # Runs before network.target so SSH and dnsmasq come up with a real address.
          procurator-netcfg = {
            description = "Apply procurator.ip=/gw=/pfx= from /proc/cmdline";
            wantedBy = ["network.target" "multi-user.target"];
            before = ["network.target"];
            after = ["systemd-udevd.service"];
            unitConfig.DefaultDependencies = false;
            serviceConfig = {
              Type = "oneshot";
              RemainAfterExit = true;
            };
            path = [pkgs.iproute2 pkgs.gawk];
            script = ''
              set -eu
              CMDLINE=$(cat /proc/cmdline)
              get() { echo "$CMDLINE" | awk -v k="$1" '{for(i=1;i<=NF;i++) if($i ~ "^"k"=") {sub("^"k"=","",$i); print $i; exit}}'; }
              IP=$(get procurator.ip)
              GW=$(get procurator.gw)
              PFX=$(get procurator.pfx)
              if [ -z "$IP" ] || [ -z "$GW" ] || [ -z "$PFX" ]; then
                echo "procurator-netcfg: missing procurator.ip/gw/pfx on cmdline" >&2
                exit 1
              fi
              # Wait briefly for eth0 to appear (virtio_net loads via udev in stage 2).
              for i in 1 2 3 4 5; do
                ip link show eth0 >/dev/null 2>&1 && break
                sleep 0.2
              done
              ip link set eth0 up
              ip addr replace "$IP/$PFX" dev eth0
              ip route replace default via "$GW" dev eth0
            '';
          };

          # OpenCode API Server - starts after network is configured by procurator-netcfg.
          opencode-server = {
            description = "OpenCode API Server";
            wantedBy = ["multi-user.target"];
            after = ["procurator-netcfg.service"];
            path = [pkgs.opencode pkgs.gawk];
            script = ''
              echo "opencode-server: no procurator.opencode-password in cmdline, starting without auth"
              exec ${pkgs.opencode}/bin/opencode serve --hostname 0.0.0.0 --port 4096
            '';
            serviceConfig = {
              Type = "simple";
              Restart = "always";
            };
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
