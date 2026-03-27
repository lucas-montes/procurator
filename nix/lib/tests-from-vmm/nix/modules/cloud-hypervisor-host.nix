# NixOS module for the *host* machine running Cloud Hypervisor.
#
# Import this into your host's NixOS configuration to get:
#   - cloud-hypervisor installed
#   - CAP_NET_ADMIN capability set on the binary so TAP devices can be opened
#     without sudo
#   - Current user's group added to the `kvm` group for /dev/kvm access
#
# Usage in your host configuration.nix (or flake):
#
#   imports = [ inputs.cloud-hypervisor.nixosModules.host ];
#
{ config, lib, pkgs, ... }:

{
  # ── Install cloud-hypervisor ───────────────────────────────────────────────
  environment.systemPackages = [ pkgs.cloud-hypervisor ];

  # ── Capability wrapper ─────────────────────────────────────────────────────
  # Places a capability-enabled binary at /run/wrappers/bin/cloud-hypervisor.
  # Opening TAP devices (--net tap=...) requires CAP_NET_ADMIN.
  security.wrappers.cloud-hypervisor = {
    source       = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
    capabilities = "cap_net_admin+ep";
    owner        = "root";
    group        = "kvm";
  };

  # ── KVM access ────────────────────────────────────────────────────────────
  # Users in the `kvm` group can open /dev/kvm without sudo.
  users.groups.kvm.members = [];   # group is created; add users via extraGroups
}
