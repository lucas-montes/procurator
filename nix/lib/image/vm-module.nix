# vm-module.nix — Base NixOS module for Cloud Hypervisor guest VMs
#
# This module configures a minimal NixOS system optimized for running
# inside a Cloud Hypervisor VM with direct kernel boot (vmlinux/bzImage).
#
# Features:
#   - systemd init (what CH expects)
#   - Serial console on ttyS0 (for --serial tty)
#   - SSH server with optional pubkey injection
#   - Virtio drivers in initrd
#   - Minimal footprint (no docs, no X11, no desktop)
#   - Parameterizable extra packages
#   - Workload entrypoint with optional auto-shutdown
#   - Arbitrary file injection into the guest image
#
# This module is imported by the flake's mkVmImage function.
# Users don't need to touch this file — they pass options via mkVmImage.

{ config, pkgs, lib, modulesPath, ... }:

let
  cfg = config.ch-vm;
in
{
  # ── Options ──────────────────────────────────────────────────────────
  # These are the knobs users can turn via mkVmImage / mkVmFromDrv.

  options.ch-vm = {
    extraPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [];
      description = "Additional packages to install in the VM image.";
      example = lib.literalExpression "[ pkgs.python3 pkgs.curl pkgs.htop ]";
    };

    sshAuthorizedKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        SSH public keys to authorize for the root user.
        If empty, only password auth is available (root/root).
      '';
      example = [ "ssh-ed25519 AAAA..." ];
    };

    hostname = lib.mkOption {
      type = lib.types.str;
      default = "ch-vm";
      description = "Hostname for the VM.";
    };

    # ── Workload entrypoint ────────────────────────────────────────
    entrypoint = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Shell command to run as the VM workload after boot.
        When set, a systemd service (vm-workload.service) starts after
        multi-user.target and executes this command as root.

        The command runs in a shell with the system PATH, so store
        paths and normal commands both work.

        Examples:
          "/nix/store/...-myapp/bin/myapp --serve"
          "python3 /opt/workload/main.py"
      '';
      example = "/nix/store/...-myapp/bin/myapp --port 8080";
    };

    autoShutdown = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Whether to power off the VM when the entrypoint exits.
        Only meaningful when `entrypoint` is set.

        When true: VM shuts down cleanly after workload exits (any exit code).
        When false: VM stays running (SSH still accessible for debugging).

        Useful for batch jobs, CI tasks, or one-shot workloads where the
        node should reclaim resources after the job finishes.
      '';
    };

    files = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = ''
        Arbitrary files to inject into the VM image.
        Keys are absolute paths in the guest; values are file contents.

        These are written at image build time via populateImageCommands,
        so they're available from the very first boot.

        Example:
          { "/etc/myapp/config.toml" = "listen = '0.0.0.0:8080'"; }
      '';
      example = lib.literalExpression ''
        {
          "/etc/myapp/config.toml" = '''
            listen = "0.0.0.0:8080"
            log_level = "info"
          ''';
          "/opt/workload/run.sh" = "#!/bin/sh\nexec python3 main.py";
        }
      '';
    };

    allowedDomains = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        List of domain names the VM is allowed to reach.
        When non-empty, guest-side nftables rules are installed at
        boot to allow only DNS + DHCP + these domains, dropping
        everything else. When empty, the VM has full internet access.

        Defense in depth:
          - Primary: guest-side nftables (this option, always active)
          - Secondary: host-side nftables (NixOS host module, production)

        Domains are resolved to IPs at boot time via getent.
      '';
      example = [ "api.openai.com" "github.com" "pypi.org" ];
    };
  };

  # ── System configuration ─────────────────────────────────────────────

  config = {
    system.stateVersion = "25.05";

    # ── Filesystem ───────────────────────────────────────────────────
    # We use an unpartitioned ext4 image, so root is /dev/vda directly.
    # The label "nixos" is set by make-ext4-fs.nix in the flake.
    # autoResize = true lets systemd-growfs expand the FS if the user
    # enlarges the raw image (e.g. truncate -s 4G nixos.raw; resize2fs).
    fileSystems."/" = {
      device = "/dev/disk/by-label/nixos";
      fsType = "ext4";
      autoResize = true;
    };

    # ── Boot ─────────────────────────────────────────────────────────
    # No bootloader — Cloud Hypervisor does direct kernel boot.
    # We pass the kernel and cmdline externally via CH's --kernel/--cmdline.
    boot = {
      # growPartition is for partitioned images (GPT). We use an
      # unpartitioned ext4 image, so disable it to avoid a failed
      # growpart service at boot. The ext4 FS auto-resizes via
      # fileSystems."/".autoResize instead.
      growPartition = false;
      kernelParams = [
        "console=ttyS0"  # Serial console for CH --serial tty
      ];
      loader.grub.enable = false;
      loader.timeout = 0;

      # Virtio drivers must be in the initrd so the kernel can find
      # the root disk and network device at boot.
      initrd.availableKernelModules = [
        "virtio_pci"
        "virtio_blk"
        "virtio_net"
        "virtio_console"
        "virtio_scsi"
      ];
    };

    # ── Networking ───────────────────────────────────────────────────
    # systemd-networkd handles DHCP on the virtio-net interface.
    # The host runs dnsmasq to serve DHCP on the bridge.
    #
    # Defense in depth: guest-side nftables is the PRIMARY filter.
    # When allowedDomains is non-empty, a systemd service resolves
    # domains → IPs at boot and installs nftables output rules.
    # The host module (ch-host) adds a SECONDARY host-side layer.
    networking = {
      hostName = cfg.hostname;
      useNetworkd = true;
      # Enable nftables when we need domain filtering inside the VM.
      # When allowedDomains is empty, no firewall (full access).
      firewall.enable = (cfg.allowedDomains != []);
      nftables.enable = (cfg.allowedDomains != []);
    };

    # DHCP on any ethernet interface (virtio-net shows up as ens* or eth*)
    systemd.network.networks."10-vm" = {
      matchConfig.Type = "ether";
      networkConfig = {
        DHCP = "ipv4";
        # Accept DNS from DHCP so resolution works for allowed domains
        DNSDefaultRoute = true;
      };
    };

    # Give systemd-networkd-wait-online a timeout so that
    # network-online.target eventually fires even if no NIC is present
    # (e.g. VM started without --net on a dev machine).
    # With a NIC + dnsmasq DHCP this completes in < 2 seconds;
    # without a NIC it times out after 10s and boot continues.
    systemd.services.systemd-networkd-wait-online.serviceConfig.ExecStart = [
      ""  # clear the default ExecStart first
      "${pkgs.systemd}/lib/systemd/systemd-networkd-wait-online --timeout=10"
    ];

    # ── SSH ───────────────────────────────────────────────────────────
    # OpenSSH is the primary way users interact with the VM.
    # Password auth is enabled for convenience; key auth is preferred.
    services.openssh = {
      enable = true;
      settings = {
        PermitRootLogin = "yes";
        PasswordAuthentication = true;
      };
    };

    # ── Serial console ───────────────────────────────────────────────
    # Auto-login on serial console for quick interactive debugging
    # via `--serial tty`. In production you'd disable this.
    services.getty.autologinUser = "root";

    # ── Users ────────────────────────────────────────────────────────
    users.users.root = {
      initialPassword = "root";
      openssh.authorizedKeys.keys = cfg.sshAuthorizedKeys;
    };

    # ── Workload service ────────────────────────────────────────────
    # When ch-vm.entrypoint is set, create a systemd service that
    # runs the user's workload after the system is fully booted.
    systemd.services.vm-workload = lib.mkIf (cfg.entrypoint != null) {
      description = "VM Workload Entrypoint";
      wantedBy = [ "multi-user.target" ];
      # Wait for DHCP to complete so the workload has an IP + DNS.
      # network-online.target is safe because we configure
      # systemd-networkd-wait-online with a short timeout below —
      # if there is no NIC the timeout fires and boot continues.
      after = [ "network-online.target" "nss-lookup.target" "sshd.service" ];
      wants = [ "network-online.target" ];

      # Type=oneshot so systemd waits for it to finish (for autoShutdown).
      # RemainAfterExit=yes so `systemctl status vm-workload` shows
      # the result even after the process exits.
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        # Ensure results directory exists before running the entrypoint
        ExecStartPre = "${pkgs.coreutils}/bin/mkdir -p /var/lib/vm-results";
        # Run in a shell so pipes, env vars, and store paths all work
        ExecStart = "${pkgs.bash}/bin/bash -c ${lib.escapeShellArg cfg.entrypoint}";
        # Give workloads a reasonable env
        Environment = [
          "PATH=${lib.makeBinPath (cfg.extraPackages ++ (with pkgs; [ coreutils bash iproute2 curl ]))}"
          "HOME=/root"
          # Workloads write structured results here.
          # The host reads this from the writable disk image after VM shutdown.
          "RESULTS_DIR=/var/lib/vm-results"
        ];
        WorkingDirectory = "/root";
        StandardOutput = "journal+console";
        StandardError = "journal+console";
      };
    };

    # Auto-shutdown: if enabled, create a unit that triggers poweroff
    # when the workload service finishes (regardless of exit code).
    systemd.services.vm-workload-shutdown = lib.mkIf (cfg.entrypoint != null && cfg.autoShutdown) {
      description = "Auto-shutdown after workload completes";
      wantedBy = [ "multi-user.target" ];
      after = [ "vm-workload.service" ];
      requires = [ "vm-workload.service" ];

      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkgs.systemd}/bin/systemctl poweroff";
      };
    };

    # ── Guest-side domain allowlist firewall ─────────────────────────
    # When allowedDomains is non-empty, resolve domains → IPs at boot
    # and install nftables rules that only allow traffic to those IPs.
    # This is the PRIMARY security layer (defense in depth).
    systemd.services.vm-domain-firewall = lib.mkIf (cfg.allowedDomains != []) {
      description = "Guest-side domain allowlist firewall";
      wantedBy = [ "multi-user.target" ];
      # We need DNS to resolve the allowed domains → IPs.
      # network-online.target ensures DHCP (and thus DNS) is ready.
      # The 10s timeout on systemd-networkd-wait-online prevents hangs.
      after = [ "network-online.target" "nss-lookup.target" ];
      wants = [ "network-online.target" ];
      before = [ "vm-workload.service" ];

      path = with pkgs; [ nftables glibc.bin coreutils gawk ];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };

      script = ''
        set -euo pipefail

        TABLE="pcr_domain_filter"

        # Flush and recreate
        nft delete table inet "$TABLE" 2>/dev/null || true
        nft add table inet "$TABLE"

        # Output chain: filter traffic LEAVING the VM
        nft add chain inet "$TABLE" output '{ type filter hook output priority 0; policy accept; }'

        # Always allow loopback
        nft add rule inet "$TABLE" output oifname "lo" accept

        # Always allow DNS (need it for resolution)
        nft add rule inet "$TABLE" output udp dport 53 accept
        nft add rule inet "$TABLE" output tcp dport 53 accept

        # Allow DHCP (need it for IP assignment)
        nft add rule inet "$TABLE" output udp dport 67 accept
        nft add rule inet "$TABLE" output udp sport 68 accept

        # Allow established/related (return traffic)
        nft add rule inet "$TABLE" output ct state established,related accept

        # Resolve each allowed domain and permit its IPs
        ${lib.concatMapStringsSep "\n" (domain: ''
          echo "Resolving ${domain}..."
          for ip in $(getent ahostsv4 "${domain}" 2>/dev/null | awk '{print $1}' | sort -u); do
            echo "  Allowing $ip (${domain})"
            nft add rule inet "$TABLE" output ip daddr "$ip" accept
          done
          # Also try IPv6
          for ip in $(getent ahostsv6 "${domain}" 2>/dev/null | awk '{print $1}' | sort -u); do
            echo "  Allowing $ip (${domain}) [v6]"
            nft add rule inet "$TABLE" output ip6 daddr "$ip" accept
          done
        '') cfg.allowedDomains}

        # Drop everything else going out
        nft add rule inet "$TABLE" output drop

        echo "Guest-side domain allowlist active: ${toString (builtins.length cfg.allowedDomains)} domains"
        nft list table inet "$TABLE"
      '';
    };

    # ── Packages ─────────────────────────────────────────────────────
    # Baseline utilities + whatever the user requested via mkVmImage.
    environment.systemPackages = with pkgs; [
      # Minimal baseline for a usable system
      coreutils
      bash
      iproute2       # ip addr, ip route — essential for network debugging
      iputils        # ping
      curl           # HTTP client for testing connectivity
      openssh        # ssh/scp client (server is separate)
    ] ++ (lib.optionals (cfg.allowedDomains != []) [
      nftables       # nft CLI for domain filtering
    ]) ++ cfg.extraPackages;

    # ── Minimization ─────────────────────────────────────────────────
    # Strip everything we don't need to keep the image small.
    # A minimal NixOS + SSH image is ~500-700MB with these settings.
    documentation.enable = false;       # No man pages, info, doc
    documentation.man.enable = false;
    documentation.nixos.enable = false;
    programs.command-not-found.enable = false;
    system.switch.enable = false;       # No nixos-rebuild in the VM
    # Keep nix CLI available (needed for first-boot store registration)
    # but disable the daemon — no rebuilding inside the VM.
    nix.settings.experimental-features = [ "nix-command" ];

    # Disable unnecessary services
    security.polkit.enable = false;
    services.udisks2.enable = false;
    xdg.icons.enable = false;
    xdg.mime.enable = false;
    xdg.sounds.enable = false;
  };
}
