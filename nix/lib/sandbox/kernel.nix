# kernel.nix — Minimal kernel configuration for Cloud Hypervisor sandbox VMs.
#
# Uses the stock NixOS kernel as a base and applies a structured config
# overlay that disables unnecessary features. This is more robust than
# linuxManualConfig because the NixOS kernel handles all kconfig defaults.
#
# The result is still slimmer than stock — we disable sound, USB, wireless,
# DRM, Bluetooth, media, and most hardware drivers that don't exist in a VM.
#
# Usage:
#   sandboxKernel = import ./kernel.nix { inherit pkgs; };
#   boot.kernelPackages = pkgs.linuxPackagesFor sandboxKernel;

{ pkgs }:

let
  lib = pkgs.lib;
  inherit (lib.kernel) yes no freeform;
in
  pkgs.linux_6_6.override {
    structuredExtraConfig = {
      # ── Identify this kernel ────────────────────────────────────
      LOCALVERSION = freeform "-sandbox";

      # ── Virtio (core CH requirement — ensure built-in, not module) ─
      VIRTIO = yes;
      VIRTIO_PCI = yes;
      VIRTIO_BLK = yes;
      VIRTIO_NET = yes;
      VIRTIO_CONSOLE = yes;
      VIRTIO_BALLOON = yes;
      VIRTIO_MMIO = yes;
      HW_RANDOM_VIRTIO = yes;
      SCSI_VIRTIO = yes;

      # ── KVM guest optimizations ─────────────────────────────────
      HYPERVISOR_GUEST = yes;
      PARAVIRT = yes;
      KVM_GUEST = yes;

      # ── Netfilter (for nftables domain allowlisting) ────────────
      NETFILTER = yes;
      NF_CONNTRACK = yes;
      NF_TABLES = yes;
      NF_TABLES_INET = yes;
      NFT_CT = yes;
      NFT_META = yes;
      NFT_COUNTER = yes;
      NFT_LOG = yes;
      NFT_REJECT = yes;

      # ── Disable hardware we don't have in a VM ─────────────────
      SOUND = lib.mkForce no;
      USB_SUPPORT = lib.mkForce no;
      WIRELESS = lib.mkForce no;
      CFG80211 = lib.mkForce no;
      DRM = lib.mkForce no;
      BT = lib.mkForce no;
      RC_CORE = lib.mkForce no;
      MEDIA_SUPPORT = lib.mkForce no;
      PCCARD = lib.mkForce no;
      ACCESSIBILITY = lib.mkForce no;
      STAGING = lib.mkForce no;
      INPUT_TOUCHSCREEN = lib.mkForce no;
      INPUT_TABLET = lib.mkForce no;
      INPUT_JOYDEV = lib.mkForce no;
      INPUT_MOUSEDEV = lib.mkForce no;

      # ── Reduce debug info ──────────────────────────────────────
      DEBUG_INFO_NONE = yes;
      DEBUG_KERNEL = lib.mkForce no;
    };
  }
