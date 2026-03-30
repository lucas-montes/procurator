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
    # Create the bridge (no physical ports). TAPs are attached at runtime.
    networking.bridges.br0.interfaces = [];

    # Assign the configured address to the bridge.
    networking.interfaces.br0.ipv4.addresses = [
      {
        address = cfg.bridgeAddress;
        prefixLength = cfg.bridgePrefixLength;
      }
    ];

    # Kernel forwarding required for NAT.
    boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

    # NAT: masquerade VM traffic through the configured external interface.
    networking.nat = {
      enable = true;
      internalInterfaces = ["br0"];
      externalInterface = cfg.externalInterface;
    };

    # dnsmasq for DHCP and DNS forwarding on the bridge. No domain filtering here.
    services.dnsmasq = {
      enable = true;
      settings = {
        interface = "br0";
        bind-interfaces = true;
        dhcp-range = cfg.dhcpRange;
        server = cfg.dnsServers;
      };
    };
  };
}
