# launch-script.nix — Generate a shell script to launch a sandbox VM
#                     with cloud-hypervisor.
#
# The script handles:
#   1. Creating a writable copy of the rootfs (the Nix store copy is read-only)
#   2. Creating a TAP device and attaching it to a bridge
#   3. Starting cloud-hypervisor with the correct kernel/initrd/disk/cmdline
#   4. Cleaning up TAP device and writable disk on exit
#
# The script is self-contained — it can be run from any app that spawns
# processes (e.g., the procurator worker, a CLI, a CI runner).
#
# Usage:
#   nix run .#sandbox-launch
#   # or: ./result/bin/launch-sandbox
#
# Environment variables (all optional):
#   SANDBOX_VM_DIR     — base directory for VM runtime files (default: /tmp/sandbox-vms)
#   SANDBOX_BRIDGE     — bridge name (default: chbr0, must exist already)
#   SANDBOX_SERIAL     — serial mode: "tty" for interactive, "file" for log (default: tty)

{ pkgs
, vmSpec
, image
, kernelPath
, initrdPath
, diskImagePath
, cmdline
, cpu
, memoryMb
, hostname
}:

let
  lib = pkgs.lib;

  # Packages needed by the launch script
  runtimeDeps = with pkgs; [
    cloud-hypervisor
    coreutils
    iproute2
    util-linux  # uuidgen
    gawk
  ];

in pkgs.writeShellScriptBin "launch-sandbox" ''
  set -euo pipefail

  # ── Configuration ────────────────────────────────────────────────
  VM_DIR="''${SANDBOX_VM_DIR:-/tmp/sandbox-vms}"
  BRIDGE="''${SANDBOX_BRIDGE:-chbr0}"
  SERIAL_MODE="''${SANDBOX_SERIAL:-tty}"
  VM_ID="$(${pkgs.util-linux}/bin/uuidgen)"
  VM_RUNTIME="$VM_DIR/$VM_ID"

  KERNEL="${kernelPath}"
  INITRD="${initrdPath}"
  DISK_ORIG="${diskImagePath}"
  CMDLINE="${cmdline}"
  CPUS=${toString cpu}
  MEMORY_MB=${toString memoryMb}

  TAP_NAME="sb-$(echo "$VM_ID" | cut -c1-10)"
  SOCKET="$VM_RUNTIME/ch.sock"

  export PATH="${lib.makeBinPath runtimeDeps}:$PATH"

  info()  { echo "[sandbox] $*"; }
  warn()  { echo "[sandbox] WARN: $*" >&2; }
  fail()  { echo "[sandbox] FAIL: $*" >&2; exit 1; }

  # ── Setup runtime directory ──────────────────────────────────────
  info "VM ID: $VM_ID"
  info "Runtime dir: $VM_RUNTIME"
  mkdir -p "$VM_RUNTIME"

  # ── Create writable disk copy ────────────────────────────────────
  # The Nix store image is read-only. We need a read-write copy so
  # the VM can write to its root filesystem.
  DISK="$VM_RUNTIME/rootfs.raw"
  info "Creating writable rootfs copy..."
  cp "$DISK_ORIG" "$DISK"
  chmod u+rw "$DISK"

  # ── Create serial log path ──────────────────────────────────────
  SERIAL_LOG="$VM_RUNTIME/serial.log"

  # ── Cleanup function ─────────────────────────────────────────────
  cleanup() {
    info "Cleaning up..."
    # Kill CH if still running
    if [ -S "$SOCKET" ]; then
      curl --unix-socket "$SOCKET" -s -X PUT \
        "http://localhost/api/v1/vmm.shutdown" 2>/dev/null || true
      sleep 1
    fi

    # Remove TAP device
    if ip link show "$TAP_NAME" &>/dev/null; then
      ip link set "$TAP_NAME" down 2>/dev/null || true
      ip tuntap del "$TAP_NAME" mode tap 2>/dev/null || true
      info "TAP $TAP_NAME removed"
    fi

    # Remove runtime dir (writable disk copy, socket, logs)
    if [ -d "$VM_RUNTIME" ]; then
      rm -rf "$VM_RUNTIME"
      info "Runtime dir cleaned"
    fi

    info "Cleanup complete"
  }
  trap cleanup EXIT

  # ── Create TAP device ───────────────────────────────────────────
  # Check if bridge exists
  if ip link show "$BRIDGE" &>/dev/null; then
    info "Creating TAP $TAP_NAME on bridge $BRIDGE"
    ip tuntap add "$TAP_NAME" mode tap
    ip link set "$TAP_NAME" master "$BRIDGE" up
    NET_ARG="--net tap=$TAP_NAME"
    info "TAP $TAP_NAME attached to $BRIDGE"
  else
    warn "Bridge $BRIDGE does not exist — VM will have no network"
    warn "To create a bridge: ip link add $BRIDGE type bridge; ip addr add 192.168.249.1/24 dev $BRIDGE; ip link set $BRIDGE up"
    NET_ARG=""
  fi

  # ── Serial configuration ─────────────────────────────────────────
  if [ "$SERIAL_MODE" = "tty" ]; then
    SERIAL_ARG="--serial tty"
    CONSOLE_ARG="--console off"
  else
    SERIAL_ARG="--serial file=$SERIAL_LOG"
    CONSOLE_ARG="--console off"
  fi

  # ── Launch cloud-hypervisor ──────────────────────────────────────
  info "Launching cloud-hypervisor..."
  info "  Kernel:  $KERNEL"
  info "  Initrd:  $INITRD"
  info "  Disk:    $DISK"
  info "  CPUs:    $CPUS"
  info "  Memory:  ''${MEMORY_MB}MB"
  info "  Socket:  $SOCKET"
  info "  Serial:  $SERIAL_MODE"

  # Build the CH command
  CH_CMD=(
    cloud-hypervisor
    --api-socket "$SOCKET"
    --kernel "$KERNEL"
    --initramfs "$INITRD"
    --disk path="$DISK"
    --cmdline "$CMDLINE"
    --cpus boot="$CPUS"
    --memory size="''${MEMORY_MB}M"
    $SERIAL_ARG
    $CONSOLE_ARG
  )

  # Add network if available
  if [ -n "''${NET_ARG:-}" ]; then
    CH_CMD+=($NET_ARG)
  fi

  info "Command: ''${CH_CMD[*]}"
  info "──────────────────────────────────────────────────"

  # Run CH — this blocks until the VM shuts down
  exec "''${CH_CMD[@]}"
''
