{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.procurator.vmm;
in {
  options.services.procurator.vmm = {
    enable = mkEnableOption "Enable procurator VMM host networking";

    externalInterface = mkOption {
      type = types.str;
      default = "wlp98s0";
      description = "Uplink interface used for NAT (change to your host uplink).";
    };

    bridgeAddress = mkOption {
      type = types.str;
      default = "192.168.100.1";
      description = "IPv4 address for the VM bridge (gateway).";
    };

    bridgePrefixLength = mkOption {
      type = types.int;
      default = 24;
      description = "Prefix length for the bridge address (usually 24).";
    };

    dhcpRange = mkOption {
      type = types.str;
      default = "192.168.100.10,192.168.100.100,12h";
      description = "DHCP range for VMs attached to the bridge.";
    };

    dnsServers = mkOption {
      type = types.listOf types.str;
      default = ["1.1.1.1" "8.8.8.8"];
      description = "Upstream DNS servers for VMs.";
    };
  };

  config = mkIf cfg.enable {
    networking = {
      # Create the bridge (no physical ports). TAPs are attached at runtime.
      bridges.br0.interfaces = [];

      # Assign the configured address to the bridge.
      interfaces.br0.ipv4.addresses = [
        {
          address = cfg.bridgeAddress;
          prefixLength = cfg.bridgePrefixLength;
        }
      ];
      # NAT: masquerade VM traffic through the configured external interface.
      nat = {
        enable = true;
        internalInterfaces = ["br0"];
        externalInterface = cfg.externalInterface;
      };

      # Trust br0 in the firewall — VMs need to reach the host for DHCP (udp/67)
      # and DNS (udp/53, tcp/53). This is safe because only our VMs are on this bridge.
      firewall.trustedInterfaces = ["br0"];
    };

    # Kernel forwarding required for NAT.
    boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

    # dnsmasq for DHCP and DNS forwarding on the bridge. No domain filtering here.
    services.dnsmasq = {
      enable = true;
      settings = {
        interface = "br0";
        # bind-interfaces = true;
        # bind-dynamic: attaches when br0 is ready, avoids silent bind failures
        # that occur with bind-interfaces if br0 gets its IP after dnsmasq starts.
        bind-dynamic = true;
        dhcp-range = cfg.dhcpRange;
        # Without this the lease has no gateway → guest ip route is empty.
        dhcp-option = "option:router,${cfg.bridgeAddress}";
        server = cfg.dnsServers;
        # Don't read host resolv.conf — only forward to servers listed above.
        no-resolv = true;
        log-dhcp = true; # helps debugging; can remove once working
      };
    };
  };
}
