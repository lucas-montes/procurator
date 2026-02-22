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
        When non-empty, the generated network setup script will
        configure iptables to only allow DNS + these domains.
        When empty, the VM has full internet access.

        This is a declarative specification — the actual iptables
        rules are applied host-side by the generated setup-network
        script (available in the build output as `networkSetupScript`).
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
    # The host-side script (setup-network.sh) runs dnsmasq to serve
    # DHCP on the bridge. Domain allowlisting is also host-side.
    networking = {
      hostName = cfg.hostname;
      useNetworkd = true;
      # Disable firewall inside the VM — filtering is done on the host.
      firewall.enable = false;
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
      after = [ "multi-user.target" "network-online.target" "sshd.service" ];
      wants = [ "network-online.target" ];

      # Type=oneshot so systemd waits for it to finish (for autoShutdown).
      # RemainAfterExit=yes so `systemctl status vm-workload` shows
      # the result even after the process exits.
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        # Run in a shell so pipes, env vars, and store paths all work
        ExecStart = "${pkgs.bash}/bin/bash -c ${lib.escapeShellArg cfg.entrypoint}";
        # Give workloads a reasonable env
        Environment = [
          "PATH=${lib.makeBinPath (cfg.extraPackages ++ (with pkgs; [ coreutils bash iproute2 curl ]))}"
          "HOME=/root"
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
    ] ++ cfg.extraPackages;

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
