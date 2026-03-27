# host.nix — NixOS module for the *host* machine running Cloud Hypervisor VMs.
#
# Import this into your host's NixOS configuration:
#
#   inputs.cloud-hypervisor-lib.nixosModules.host
#
# What it provides:
#   - cloud-hypervisor installed system-wide
#   - /run/wrappers/bin/cloud-hypervisor with CAP_NET_ADMIN+ep so TAP devices
#     can be opened without sudo
#   - A Linux bridge `br0` that VMs can attach their TAP interfaces to,
#     giving each VM a real IP on the host's network
#   - The `kvm` group so non-root users can open /dev/kvm
#
# Bridge setup notes:
#   - br0 is configured with DHCP so it gets an IP from your upstream router.
#   - Attach your uplink NIC to br0 by setting `cloudHypervisor.host.uplinkInterface`.
#   - VMs use a TAP device (created automatically by cloud-hypervisor) which is
#     enslaved to br0, giving the VM an IP on the same subnet as the host.
#
# Example usage:
#
#   { inputs, ... }: {
#     imports = [ inputs.cloud-hypervisor-lib.nixosModules.host ];
#
#     cloudHypervisor.host.uplinkInterface = "eth0";  # your NIC name
#
#     users.users.alice.extraGroups = [ "kvm" ];
#   }

{ config, lib, pkgs, ... }:

let
  cfg = config.cloudHypervisor.host;
in
{
  # ── Option declarations ────────────────────────────────────────────────────
  options.cloudHypervisor.host = {

    uplinkInterface = lib.mkOption {
      type    = lib.types.str;
      default = "eth0";
      description = ''
        Name of the physical NIC to attach to br0.
        VMs bridged to br0 will appear on the same L2 network as this interface.
      '';
    };

    bridgeName = lib.mkOption {
      type    = lib.types.str;
      default = "br0";
      description = "Name of the Linux bridge interface created on the host.";
    };

    enableBridge = lib.mkOption {
      type    = lib.types.bool;
      default = true;
      description = ''
        Whether to create and manage the bridge interface.
        Disable if you manage bridging outside of NixOS (e.g. via netplan).
      '';
    };
  };

  # ── Implementation ─────────────────────────────────────────────────────────
  config = {

    # ── cloud-hypervisor ──────────────────────────────────────────────────
    environment.systemPackages = [ pkgs.cloud-hypervisor ];

    # Capability wrapper — gives CAP_NET_ADMIN to the binary so TAP devices
    # can be created without running cloud-hypervisor as root.
    security.wrappers.cloud-hypervisor = {
      source       = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
      capabilities = "cap_net_admin+ep";
      owner        = "root";
      group        = "kvm";
    };

    # ── KVM access ────────────────────────────────────────────────────────
    users.groups.kvm = {};
    services.udev.extraRules = ''
      KERNEL=="kvm", GROUP="kvm", MODE="0660"
    '';

    # ── Bridge networking ─────────────────────────────────────────────────
    networking.bridges = lib.mkIf cfg.enableBridge {
      ${cfg.bridgeName}.interfaces = [ cfg.uplinkInterface ];
    };

# https://alberand.com/nixos-linux-kernel-vm.html#custom-kernel-and-config
    # This goes into your host configuration.nix
# networking.interfaces.tap0 = {
#   name = "tap0";
#   virtual = true;
#   virtualType = "tap";
#   virtualOwner = "alberand";
# };

# networking.interfaces.tap0 = {
#   ipv4 = {
#     addresses = [{
#       address = "192.168.10.1";
#       prefixLength = 16;
#     }];
#   };
# };

# This goes into your vm.nix
# networking.interfaces.eth1 = {
#   ipv4.addresses = [{
#     address = "192.168.10.2";
#     prefixLength = 24;
#   }];
# };

    systemd.network = lib.mkIf cfg.enableBridge {
      enable = true;

      # Bridge interface — get IP via DHCP
      networks."20-${cfg.bridgeName}" = {
        matchConfig.Name = cfg.bridgeName;
        networkConfig.DHCP = "ipv4";
      };

      # Uplink NIC — enslaved to the bridge, no IP of its own
      networks."10-uplink" = {
        matchConfig.Name = cfg.uplinkInterface;
        networkConfig.Bridge = cfg.bridgeName;
      };

      # TAP interfaces created by cloud-hypervisor are automatically
      # enslaved to br0 by the kernel when --net tap=tapX,... is used
      # together with the bridge. The bridge name must match bridgeName above.
      networks."30-vm-tap" = {
        matchConfig.Name = "tap*";
        networkConfig.Bridge = cfg.bridgeName;
      };
    };

    # Forward traffic between bridge ports (needed for VM ↔ host and VM ↔ WAN)
    boot.kernel.sysctl = lib.mkIf cfg.enableBridge {
      "net.ipv4.ip_forward"              = 1;
      "net.bridge.bridge-nf-call-iptables" = 0;
    };
  };
}
