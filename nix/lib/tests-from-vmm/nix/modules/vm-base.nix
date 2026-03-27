# vm-base.nix — base NixOS module that runs *inside* a Cloud Hypervisor VM.
#
# Provides:
#   - virtio kernel modules + hvc0 console (direct kernel boot)
#   - dedicated non-root user with SSH public key
#   - systemd-networkd DHCP on the virtio NIC
#   - OpenSSH (key-only, no passwords)
#   - coreutils + a user-supplied extra package list
#   - unbound local resolver enforcing a domain allowlist
#   - nftables rules that block all outbound DNS except to unbound
#
# This module is parameterised via the options defined below.
# It is imported automatically by makeVm — users do not need to import it
# directly unless they want to compose it manually.

{ config, lib, pkgs, ... }:

let
  cfg = config.cloudHypervisor.vm;
in
{
  # ── Option declarations ────────────────────────────────────────────────────
  options.cloudHypervisor.vm = {

    username = lib.mkOption {
      type    = lib.types.str;
      default = "vm-user";
      description = "Name of the dedicated non-root user created in the VM.";
    };

    sshKey = lib.mkOption {
      type    = lib.types.str;
      default = "";
      description = "SSH public key string authorised for the dedicated user.";
    };

    allowedDomains = lib.mkOption {
      type    = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        Domains that the VM is allowed to resolve and reach.
        All other DNS queries return NXDOMAIN.
        When the list is empty every domain is allowed (no filtering).
      '';
    };

    extraPackages = lib.mkOption {
      type    = lib.types.functionTo (lib.types.listOf lib.types.package);
      default = _: [];
      description = "Function pkgs -> list of extra packages to install in the VM.";
    };

    diskSize = lib.mkOption {
      type    = lib.types.str;
      default = "4G";
      description = "Size of the root disk image (passed to systemd-repart).";
    };
  };

  # ── Implementation ─────────────────────────────────────────────────────────
  config = {

    # ── Kernel / boot ──────────────────────────────────────────────────────
    boot.initrd.availableKernelModules = [
      "virtio" "virtio_pci" "virtio_mmio"
      "virtio_blk" "virtio_net" "virtio_console" "virtio_rng"
      "virtiofs" "9p" "9pnet_virtio"
    ];
    boot.kernelModules     = [ "virtio_pci" "virtio_blk" "virtio_net" "virtio_console" ];
    boot.kernelParams      = [ "console=hvc0" "console=ttyS0" ];
    boot.loader.grub.enable          = lib.mkForce false;
    boot.loader.systemd-boot.enable  = lib.mkForce false;

    fileSystems."/" = {
      device = "/dev/vda";
      fsType = "ext4";
    };

    # ── User ──────────────────────────────────────────────────────────────
    users.users.${cfg.username} = {
      isNormalUser = true;
      extraGroups  = [ "wheel" ];
      openssh.authorizedKeys.keys = lib.optional (cfg.sshKey != "") cfg.sshKey;
    };
    users.users.root.hashedPassword = "!";

    security.sudo.extraRules = [{
      users    = [ cfg.username ];
      commands = [{ command = "ALL"; options = [ "NOPASSWD" ]; }];
    }];

    # ── SSH ───────────────────────────────────────────────────────────────
    services.openssh = {
      enable = true;
      settings = {
        PasswordAuthentication          = false;
        PermitRootLogin                 = "no";
        ChallengeResponseAuthentication = false;
      };
    };

    # ── Network ───────────────────────────────────────────────────────────
    # Use systemd-networkd for DHCP on the virtio NIC (eth0 / ens*)
    networking.useDHCP                      = false;
    networking.usePredictableInterfaceNames = false;
    systemd.network.enable                  = true;
    systemd.network.networks."10-eth" = {
      matchConfig.Name = "eth* en*";
      networkConfig = {
        DHCP           = "ipv4";
        DNS            = "127.0.0.1";   # point at local unbound
      };
      dhcpV4Config.UseDNS = false;      # we manage DNS ourselves
    };

    # ── DNS allowlist (unbound) ───────────────────────────────────────────
    services.unbound = {
      enable   = true;
      settings = {
        server = {
          interface           = [ "127.0.0.1" ];
          access-control      = [ "127.0.0.0/8 allow" ];
          do-not-query-localhost = false;

          # When allowedDomains is non-empty, block everything by default
          # then add local-zone overrides to allow each listed domain.
        } // lib.optionalAttrs (cfg.allowedDomains != []) {
          # Catch-all: refuse all queries
          local-zone = [ ''"." refuse'' ]
            # Then allow each listed domain (and its subdomains)
            ++ map (d: ''"${d}." transparent'') cfg.allowedDomains;
        };

        forward-zone = [{
          name    = ".";
          forward-addr = [
            "1.1.1.1"
            "8.8.8.8"
          ];
        }];
      };
    };

    # ── Firewall / nftables ───────────────────────────────────────────────
    # Block all outbound DNS (UDP+TCP port 53) except to 127.0.0.1 (unbound).
    # This forces all DNS through our allowlist resolver.
    networking.nftables.enable = true;
    networking.nftables.tables.ch-dns-filter = {
      family = "inet";
      content = ''
        chain output-dns {
          type filter hook output priority 0; policy accept;

          # Allow DNS to local unbound
          ip daddr 127.0.0.1 udp dport 53 accept
          ip daddr 127.0.0.1 tcp dport 53 accept

          # Block all other outbound DNS
          udp dport 53 drop
          tcp dport 53 drop
        }
      '';
    };

    # ── Packages ──────────────────────────────────────────────────────────
    environment.systemPackages =
      (with pkgs; [
        bash
        coreutils
        curl
        wget
        iproute2
        iputils
        procps
        util-linux
        vim
      ])
      ++ cfg.extraPackages pkgs;

    # ── Misc ──────────────────────────────────────────────────────────────
    # hvc0 getty for local console access
    systemd.services."serial-getty@hvc0" = {
      enable   = true;
      wantedBy = [ "multi-user.target" ];
    };

    documentation.enable     = false;
    documentation.man.enable = false;
    system.stateVersion      = "25.05";
  };
}
