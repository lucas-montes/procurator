{
  config,
  lib,
  ...
}:
with lib;
let
  cfg = config.services.procurator.vmm;
in
{
  options.services.procurator.vmm = {
    enable = mkEnableOption "Enable procurator VMM host networking";

    externalInterface = mkOption {
      type = types.str;
      default = "wlp98s0";
      description = "Uplink interface used for NAT (change to your host uplink).";
    };

    bridgeName = mkOption {
      type = types.str;
      default = "br0";
      description = "Name of the bridge.";
    };

    bridgeAddress = mkOption {
      type = types.str;
      default = "10.0.0.1";
      description = "IPv4 address for the VM bridge (gateway). Must be the .1 of the worker IP pool subnet.";
    };

    bridgePrefixLength = mkOption {
      type = types.int;
      default = 8;
      description = "Prefix length for the bridge address. 8 = /8, giving ~16 million usable addresses.";
    };

    dnsServers = mkOption {
      type = types.listOf types.str;
      default = [
        "1.1.1.1"
        "8.8.8.8"
      ];
      description = "Upstream DNS servers the host dnsmasq forwards VM queries to.";
    };

    environment = mkOption {
      type = types.enum [
        "dev"
        "staging"
        "prod"
      ];
      default = "dev";
      description = "Deployment environment. `prod` blocks dev-only settings from being accidentally enabled.";
    };

    dnsWildcardDomain = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "worker.local";
      description = ''
        If set, dnsmasq resolves `*.<domain>` to `127.0.0.1` so that
        `<vm-id>.<domain>` reaches the proxy running on localhost.

        This is a **dev-only** setting. Setting it together with
        `environment = "prod"` will cause a build failure.
      '';
    };
  };

  config = mkIf cfg.enable {
    # ── Environment safety checks ───────────────────────────────────────
    assertions = [
      {
        assertion = cfg.environment != "prod" || cfg.dnsWildcardDomain == null;
        message = ''
          services.procurator.vmm.dnsWildcardDomain is set to "${toString cfg.dnsWildcardDomain}"
          but environment is "prod". This is a development-only setting that
          resolves *.<domain> to 127.0.0.1 via dnsmasq. Remove dnsWildcardDomain
          or set services.procurator.vmm.environment to "dev" or "staging".
        '';
      }
    ];
    # ── Device permissions ──────────────────────────────────────────────
    # Ensure /dev/net/tun and /dev/kvm are group-accessible so the
    # unprivileged worker (via kvm/netdev group membership) can open them.
    # /dev/vhost-net is optional but used by CH for vhost acceleration.
    #
    # NixOS already ships kvm udev rules in most kernels, but we add
    # explicit ones to guarantee correctness on all configurations.
    services.udev.extraRules = ''
      # /dev/kvm — hardware virtualisation (group kvm, rw)
      KERNEL=="kvm", GROUP="kvm", MODE="0660"
      # /dev/net/tun — TAP device creation (group netdev, rw)
      KERNEL=="tun", GROUP="netdev", MODE="0660"
      # /dev/vhost-net — vhost-net acceleration (group kvm, rw)
      KERNEL=="vhost-net", GROUP="kvm", MODE="0660"
    '';

    # TODO: add user to the groups
    # users.users.<worker-user>.extraGroups = [ "kvm" "netdev" ];

    networking = {
      # Create the bridge (no physical ports). TAPs are attached at runtime.
      bridges.${cfg.bridgeName}.interfaces = [ ];

      # Assign the configured address to the bridge.
      interfaces.${cfg.bridgeName}.ipv4.addresses = [
        {
          address = cfg.bridgeAddress;
          prefixLength = cfg.bridgePrefixLength;
        }
      ];
      # NAT: masquerade VM traffic through the configured external interface.
      nat = {
        enable = true;
        internalInterfaces = [ cfg.bridgeName ];
        externalInterface = cfg.externalInterface;
      };

      # Trust br0 in the firewall — VMs need to reach the host for DNS (udp/53, tcp/53).
      # DHCP is NOT served on this bridge: guests receive their IP at boot from
      # procurator.ip=/gw=/pfx= tokens the worker appends to the kernel cmdline
      # (parsed by the procurator-netcfg systemd unit inside the image).
      # This is safe because only our VMs are on this bridge.
      firewall.trustedInterfaces = [ cfg.bridgeName ];
    };

    # Kernel forwarding required for NAT.
    boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

    # dnsmasq: DNS forwarder for VMs via the bridge, plus optional dev wildcard.
    # Guests are given their IP statically by the cmdline parameter that the worker
    # appends at VM-create time. dnsmasq here answers DNS queries and forwards them
    # upstream. It listens on all interfaces so the host itself can also query it.
    services.dnsmasq = {
      enable = true;
      settings = {
        # bind-dynamic: attaches when br0 is ready, avoids silent bind failures
        # that occur with bind-interfaces if br0 gets its IP after dnsmasq starts.
        bind-dynamic = true;
        # Explicitly disable DHCP: no dhcp-range, no dhcp-authoritative.
        # (dnsmasq without dhcp-range does not serve DHCP at all.)
        port = 53;
        server = cfg.dnsServers;
        # Don't read host resolv.conf — only forward to servers listed above.
        no-resolv = true;
      }
      # Dev wildcard: resolve *.<domain> to 127.0.0.1 for browser proxy access.
      // optionalAttrs (cfg.dnsWildcardDomain != null) {
        address = [ "/${cfg.dnsWildcardDomain}/127.0.0.1" ];
      };
    };
  };
}
