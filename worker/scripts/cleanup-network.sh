#!/usr/bin/env bash
# Cleanup network infrastructure for Procurator worker
# This script must be run as root or with sudo

set -euo pipefail

BRIDGE_NAME="${BRIDGE_NAME:-br-procurator}"

echo "Cleaning up Procurator network infrastructure..."

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo "This script must be run as root (use sudo)"
   exit 1
fi

# Delete all TAP devices attached to the bridge
if ip link show "$BRIDGE_NAME" &> /dev/null; then
    echo "Removing TAP devices from bridge..."

    # Find all interfaces attached to the bridge
    for iface in $(ip link show master "$BRIDGE_NAME" | grep -o 'tap-[^:]*' || true); do
        echo "  Removing $iface..."
        ip link set "$iface" nomaster || true
        ip link delete "$iface" || true
    done
fi

# Delete the bridge
if ip link show "$BRIDGE_NAME" &> /dev/null; then
    echo "Deleting bridge $BRIDGE_NAME..."
    ip link set "$BRIDGE_NAME" down
    ip link delete "$BRIDGE_NAME"
fi

# Remove iptables rules (optional)
if command -v iptables &> /dev/null; then
    echo "Removing iptables rules..."
    DEFAULT_IF=$(ip route | grep default | awk '{print $5}' | head -n1 || true)

    if [[ -n "$DEFAULT_IF" ]]; then
        iptables -t nat -D POSTROUTING -s 10.100.0.0/16 -o "$DEFAULT_IF" -j MASQUERADE 2>/dev/null || true
        iptables -D FORWARD -i "$BRIDGE_NAME" -o "$DEFAULT_IF" -j ACCEPT 2>/dev/null || true
        iptables -D FORWARD -i "$DEFAULT_IF" -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || true
    fi
fi

echo "Cleanup complete!"
