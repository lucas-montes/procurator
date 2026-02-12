#!/usr/bin/env bash
# Setup network infrastructure for Procurator worker
# This script must be run as root or with sudo

set -euo pipefail

BRIDGE_NAME="${BRIDGE_NAME:-br-procurator}"
BRIDGE_IP="${BRIDGE_IP:-10.100.0.1/16}"

echo "Setting up Procurator network infrastructure..."

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo "This script must be run as root (use sudo)"
   exit 1
fi

# Load required kernel modules
echo "Loading kernel modules..."
modprobe kvm || true
modprobe kvm_intel || modprobe kvm_amd || true
modprobe tun

# Check if bridge already exists
if ip link show "$BRIDGE_NAME" &> /dev/null; then
    echo "Bridge $BRIDGE_NAME already exists"
else
    echo "Creating bridge $BRIDGE_NAME..."
    ip link add name "$BRIDGE_NAME" type bridge
    ip link set "$BRIDGE_NAME" up
    ip addr add "$BRIDGE_IP" dev "$BRIDGE_NAME"
fi

# Enable IP forwarding
echo "Enabling IP forwarding..."
sysctl -w net.ipv4.ip_forward=1
sysctl -w net.ipv6.conf.all.forwarding=1

# Set up NAT for VM internet access (optional)
if command -v iptables &> /dev/null; then
    echo "Setting up NAT for VM internet access..."

    # Find the default network interface
    DEFAULT_IF=$(ip route | grep default | awk '{print $5}' | head -n1)

    if [[ -n "$DEFAULT_IF" ]]; then
        # Enable masquerading for VMs
        iptables -t nat -C POSTROUTING -s 10.100.0.0/16 -o "$DEFAULT_IF" -j MASQUERADE 2>/dev/null || \
            iptables -t nat -A POSTROUTING -s 10.100.0.0/16 -o "$DEFAULT_IF" -j MASQUERADE

        # Allow forwarding
        iptables -C FORWARD -i "$BRIDGE_NAME" -o "$DEFAULT_IF" -j ACCEPT 2>/dev/null || \
            iptables -A FORWARD -i "$BRIDGE_NAME" -o "$DEFAULT_IF" -j ACCEPT

        iptables -C FORWARD -i "$DEFAULT_IF" -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || \
            iptables -A FORWARD -i "$DEFAULT_IF" -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT

        echo "NAT configured for VMs to access internet via $DEFAULT_IF"
    else
        echo "Warning: Could not find default network interface, skipping NAT setup"
    fi
fi

# Set permissions on /dev/kvm
echo "Setting permissions on /dev/kvm..."
if [[ -e /dev/kvm ]]; then
    chmod 666 /dev/kvm
    echo "KVM device permissions set"
else
    echo "Warning: /dev/kvm not found. Is KVM available?"
fi

# Create directory for VM artifacts
VM_DIR="/var/lib/procurator/vms"
echo "Creating VM artifacts directory: $VM_DIR"
mkdir -p "$VM_DIR"
chmod 755 "$VM_DIR"

echo ""
echo "Network setup complete!"
echo ""
echo "Bridge: $BRIDGE_NAME"
echo "IP: $BRIDGE_IP"
echo "VM subnet: 10.100.0.0/16"
echo ""
echo "To verify setup:"
echo "  ip link show $BRIDGE_NAME"
echo "  ip addr show $BRIDGE_NAME"
echo ""
echo "To make IP forwarding persistent, add to /etc/sysctl.conf:"
echo "  net.ipv4.ip_forward = 1"
echo "  net.ipv6.conf.all.forwarding = 1"
