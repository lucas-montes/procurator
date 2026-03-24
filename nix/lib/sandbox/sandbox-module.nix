# sandbox-module.nix — Minimal NixOS module for Cloud Hypervisor sandbox VMs.
#
# Optimized for ephemeral, secure sandbox workloads:
#   - Minimal NixOS with systemd init
#   - Serial console (ttyS0) for CH --serial tty
#   - Virtio drivers for disk/net
#   - Workload entrypoint with auto-shutdown
#   - Domain allowlist firewall (guest-side nftables)
#   - Local file injection (directories copied from host)
#   - No docs, no desktop, no unnecessary services
#
# This is imported by mkSandbox. Users don't touch this directly.

{ config, pkgs, lib, modulesPath, ... }:

let
  cfg = config.ch-sandbox;
in
{
  # ── Options ──────────────────────────────────────────────────────────

  options.ch-sandbox = {
    extraPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [];
      description = "Additional packages to install in the sandbox VM.";
    };

    hostname = lib.mkOption {
      type = lib.types.str;
      default = "sandbox";
      description = "Hostname for the sandbox VM.";
    };

    sshAuthorizedKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "SSH public keys for the root user.";
    };

    entrypoint = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Shell command to run as the sandbox workload after boot.
        Examples:
          "python3 /workspace/main.py"
          "/workspace/run.sh --arg1 value"
      '';
    };

    autoShutdown = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Power off the VM when the entrypoint exits.
        Default true for sandbox workloads (batch jobs, CI, testing).
      '';
    };

    inlineFiles = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = ''
        Files to inject as string content.
        Keys are absolute guest paths; values are file contents.
      '';
    };

    localFilesPath = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Store path containing the local files tree to copy into the image.
        Set by mkSandbox — not meant to be set directly by users.
      '';
    };

    allowedDomains = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        Domain allowlist. When non-empty, guest-side nftables rules
        allow only DNS + DHCP + these domains, dropping everything else.
        When empty, the VM has full internet access.
      '';
    };
  };

  # ── Configuration ────────────────────────────────────────────────────

  config = {
    system.stateVersion = "25.05";

    # ── Filesystem ───────────────────────────────────────────────────
    fileSystems."/" = {
      device = "/dev/disk/by-label/nixos";
      fsType = "ext4";
      autoResize = true;
    };

    # ── Boot ─────────────────────────────────────────────────────────
    boot = {
      growPartition = false;
      kernelParams = [ "console=ttyS0" ];
      loader.grub.enable = false;
      loader.timeout = 0;

      initrd.availableKernelModules = [
        "virtio_pci"
        "virtio_blk"
        "virtio_net"
        "virtio_console"
        "virtio_scsi"
        "ext4"
      ];
    };

    # ── Networking ───────────────────────────────────────────────────
    networking = {
      hostName = cfg.hostname;
      useNetworkd = true;
      firewall.enable = (cfg.allowedDomains != []);
      nftables.enable = (cfg.allowedDomains != []);
    };

    # DHCP on any ethernet interface (virtio-net)
    systemd.network.networks."10-sandbox" = {
      matchConfig.Type = "ether";
      networkConfig = {
        DHCP = "ipv4";
        DNSDefaultRoute = true;
      };
    };

    # Timeout so boot doesn't hang without a NIC
    systemd.services.systemd-networkd-wait-online.serviceConfig.ExecStart = [
      ""  # clear default
      "${pkgs.systemd}/lib/systemd/systemd-networkd-wait-online --timeout=10"
    ];

    # ── SSH ───────────────────────────────────────────────────────────
    services.openssh = {
      enable = true;
      settings = {
        PermitRootLogin = "yes";
        PasswordAuthentication = true;
      };
    };

    # ── Serial console ───────────────────────────────────────────────
    services.getty.autologinUser = "root";

    # ── Users ────────────────────────────────────────────────────────
    users.users.root = {
      initialPassword = "root";
      openssh.authorizedKeys.keys = cfg.sshAuthorizedKeys;
    };

    # ── Workload entrypoint ──────────────────────────────────────────
    systemd.services.sandbox-workload = lib.mkIf (cfg.entrypoint != null) {
      description = "Sandbox Workload";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" "nss-lookup.target" "sshd.service" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStartPre = "${pkgs.coreutils}/bin/mkdir -p /var/lib/sandbox-results";
        ExecStart = "${pkgs.bash}/bin/bash -c ${lib.escapeShellArg cfg.entrypoint}";
        Environment = [
          "PATH=${lib.makeBinPath (cfg.extraPackages ++ (with pkgs; [ coreutils bash iproute2 curl ]))}"
          "HOME=/root"
          "RESULTS_DIR=/var/lib/sandbox-results"
        ];
        WorkingDirectory = "/root";
        StandardOutput = "journal+console";
        StandardError = "journal+console";
      };
    };

    # ── Auto-shutdown ────────────────────────────────────────────────
    systemd.services.sandbox-shutdown = lib.mkIf (cfg.entrypoint != null && cfg.autoShutdown) {
      description = "Auto-shutdown after sandbox workload completes";
      wantedBy = [ "multi-user.target" ];
      after = [ "sandbox-workload.service" ];
      requires = [ "sandbox-workload.service" ];

      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkgs.systemd}/bin/systemctl poweroff";
      };
    };

    # ── Guest-side domain allowlist firewall ─────────────────────────
    systemd.services.sandbox-domain-firewall = lib.mkIf (cfg.allowedDomains != []) {
      description = "Sandbox domain allowlist firewall";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" "nss-lookup.target" ];
      wants = [ "network-online.target" ];
      before = [ "sandbox-workload.service" ];

      path = with pkgs; [ nftables glibc.bin coreutils gawk ];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };

      script = ''
        set -euo pipefail

        TABLE="sandbox_domain_filter"

        # Flush and recreate
        nft delete table inet "$TABLE" 2>/dev/null || true
        nft add table inet "$TABLE"

        # Output chain: filter traffic LEAVING the VM
        nft add chain inet "$TABLE" output '{ type filter hook output priority 0; policy accept; }'

        # Always allow loopback
        nft add rule inet "$TABLE" output oifname "lo" accept

        # Always allow DNS
        nft add rule inet "$TABLE" output udp dport 53 accept
        nft add rule inet "$TABLE" output tcp dport 53 accept

        # Allow DHCP
        nft add rule inet "$TABLE" output udp dport 67 accept
        nft add rule inet "$TABLE" output udp sport 68 accept

        # Allow established/related
        nft add rule inet "$TABLE" output ct state established,related accept

        # Resolve each allowed domain and permit its IPs
        ${lib.concatMapStringsSep "\n" (domain: ''
          echo "Resolving ${domain}..."
          for ip in $(getent ahostsv4 "${domain}" 2>/dev/null | awk '{print $1}' | sort -u); do
            echo "  Allowing $ip (${domain})"
            nft add rule inet "$TABLE" output ip daddr "$ip" accept
          done
          for ip in $(getent ahostsv6 "${domain}" 2>/dev/null | awk '{print $1}' | sort -u); do
            echo "  Allowing $ip (${domain}) [v6]"
            nft add rule inet "$TABLE" output ip6 daddr "$ip" accept
          done
        '') cfg.allowedDomains}

        # Drop everything else
        nft add rule inet "$TABLE" output drop

        echo "Domain allowlist active: ${toString (builtins.length cfg.allowedDomains)} domains"
        nft list table inet "$TABLE"
      '';
    };

    # ── Packages ─────────────────────────────────────────────────────
    environment.systemPackages = with pkgs; [
      coreutils
      bash
      iproute2
      iputils
      curl
      openssh
    ] ++ (lib.optionals (cfg.allowedDomains != []) [
      nftables
    ]) ++ cfg.extraPackages;

    # ── Minimization ─────────────────────────────────────────────────
    documentation.enable = false;
    documentation.man.enable = false;
    documentation.nixos.enable = false;
    programs.command-not-found.enable = false;
    system.switch.enable = false;
    nix.settings.experimental-features = [ "nix-command" ];
    security.polkit.enable = false;
    services.udisks2.enable = false;
    xdg.icons.enable = false;
    xdg.mime.enable = false;
    xdg.sounds.enable = false;
  };
}
