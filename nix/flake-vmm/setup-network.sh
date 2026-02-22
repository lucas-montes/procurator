#!/usr/bin/env bash
# setup-network.sh — Host-side network setup for Cloud Hypervisor VMs
#
# This script creates a TAP device + bridge + dnsmasq (DHCP) so the VM
# can get an IP and reach the outside world. It also supports domain
# allowlisting: if you pass domain names, ONLY those domains will be
# reachable from the VM (everything else is blocked via iptables).
#
# Usage:
#   sudo ./setup-network.sh [--allow-domain DOMAIN]...
#
# Examples:
#   # Full internet access (no filtering):
#   sudo ./setup-network.sh
#
#   # Only allow specific domains (sandbox mode):
#   sudo ./setup-network.sh --allow-domain api.openai.com --allow-domain github.com
#
# The script outputs the TAP device name and MAC address to use with
# cloud-hypervisor's --net flag.
#
# Cleanup:
#   sudo ./setup-network.sh --cleanup
#
# Requirements: iproute2, iptables, dnsmasq, bash
# Must be run as root (needs network namespace privileges).

set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────
BRIDGE_NAME="chbr0"
TAP_NAME="chtap0"
BRIDGE_IP="192.168.249.1"
BRIDGE_SUBNET="192.168.249.0/24"
DHCP_RANGE_START="192.168.249.10"
DHCP_RANGE_END="192.168.249.50"
DNSMASQ_PID_FILE="/tmp/ch-dnsmasq.pid"
ALLOWED_DOMAINS=()
CLEANUP=false

# ── Argument parsing ─────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --allow-domain)
            ALLOWED_DOMAINS+=("$2")
            shift 2
            ;;
        --cleanup)
            CLEANUP=true
            shift
            ;;
        --tap-name)
            TAP_NAME="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--allow-domain DOMAIN]... [--tap-name NAME] [--cleanup]"
            echo ""
            echo "Options:"
            echo "  --allow-domain DOMAIN   Allow VM to reach this domain (can repeat)."
            echo "                          If none specified, VM has full internet access."
            echo "  --tap-name NAME         TAP device name (default: chtap0)"
            echo "  --cleanup               Tear down bridge, TAP, iptables, dnsmasq"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# ── Cleanup mode ─────────────────────────────────────────────────────
if $CLEANUP; then
    echo "=== Cleaning up Cloud Hypervisor networking ==="

    # Stop dnsmasq
    if [[ -f "$DNSMASQ_PID_FILE" ]]; then
        kill "$(cat "$DNSMASQ_PID_FILE")" 2>/dev/null || true
        rm -f "$DNSMASQ_PID_FILE"
        echo "  Stopped dnsmasq"
    fi

    # Remove iptables rules (best-effort, ignore errors)
    iptables -t nat -D POSTROUTING -s "$BRIDGE_SUBNET" ! -o "$BRIDGE_NAME" -j MASQUERADE 2>/dev/null || true
    iptables -D FORWARD -i "$BRIDGE_NAME" -o "$BRIDGE_NAME" -j ACCEPT 2>/dev/null || true
    iptables -D FORWARD -i "$BRIDGE_NAME" -j ACCEPT 2>/dev/null || true
    iptables -D FORWARD -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || true

    # Remove domain-allowlist chain if it exists
    iptables -D FORWARD -i "$BRIDGE_NAME" ! -o "$BRIDGE_NAME" -j CH_VM_ALLOW 2>/dev/null || true
    iptables -F CH_VM_ALLOW 2>/dev/null || true
    iptables -X CH_VM_ALLOW 2>/dev/null || true

    # Delete TAP and bridge
    ip link set "$TAP_NAME" down 2>/dev/null || true
    ip link del "$TAP_NAME" 2>/dev/null || true
    ip link set "$BRIDGE_NAME" down 2>/dev/null || true
    ip link del "$BRIDGE_NAME" 2>/dev/null || true

    echo "  Cleanup complete."
    exit 0
fi

# ── Must be root ─────────────────────────────────────────────────────
if [[ $EUID -ne 0 ]]; then
    echo "Error: This script must be run as root (sudo)." >&2
    exit 1
fi

echo "=== Setting up Cloud Hypervisor VM networking ==="

# ── Create bridge ────────────────────────────────────────────────────
# The bridge connects the TAP device to the host network stack.
# dnsmasq listens on the bridge to provide DHCP to VMs.
if ! ip link show "$BRIDGE_NAME" &>/dev/null; then
    ip link add name "$BRIDGE_NAME" type bridge
    ip addr add "$BRIDGE_IP/24" dev "$BRIDGE_NAME"
    ip link set "$BRIDGE_NAME" up
    echo "  Created bridge $BRIDGE_NAME ($BRIDGE_IP)"
else
    echo "  Bridge $BRIDGE_NAME already exists"
fi

# ── Create TAP device ───────────────────────────────────────────────
# Cloud Hypervisor attaches the VM's virtio-net to this TAP device.
if ! ip link show "$TAP_NAME" &>/dev/null; then
    ip tuntap add dev "$TAP_NAME" mode tap
    ip link set "$TAP_NAME" master "$BRIDGE_NAME"
    ip link set "$TAP_NAME" up
    echo "  Created TAP $TAP_NAME (attached to $BRIDGE_NAME)"
else
    echo "  TAP $TAP_NAME already exists"
fi

# ── Enable IP forwarding ────────────────────────────────────────────
# Required for the VM to reach the outside world via the host.
sysctl -w net.ipv4.ip_forward=1 >/dev/null
echo "  IP forwarding enabled"

# ── iptables: NAT + forwarding ──────────────────────────────────────
# MASQUERADE: rewrite VM traffic source IP to the host's outbound IP.
# FORWARD rules: allow traffic to flow through the bridge.
iptables -t nat -C POSTROUTING -s "$BRIDGE_SUBNET" ! -o "$BRIDGE_NAME" -j MASQUERADE 2>/dev/null ||
    iptables -t nat -A POSTROUTING -s "$BRIDGE_SUBNET" ! -o "$BRIDGE_NAME" -j MASQUERADE

# ── Domain allowlisting (sandbox mode) ──────────────────────────────
# If --allow-domain was passed, we create an iptables chain that:
#   1. Allows DNS (so the VM can resolve the allowed domains)
#   2. Allows only IPs of the specified domains
#   3. Drops everything else
#
# This is the network sandbox: the VM can ONLY reach the listed domains.
# Without --allow-domain, the VM has full internet access.
if [[ ${#ALLOWED_DOMAINS[@]} -gt 0 ]]; then
    echo "  Setting up domain allowlist (sandbox mode):"

    # Create a custom chain for allowlisting
    iptables -N CH_VM_ALLOW 2>/dev/null || iptables -F CH_VM_ALLOW

    # Always allow DNS (port 53) so domain resolution works
    iptables -A CH_VM_ALLOW -p udp --dport 53 -j ACCEPT
    iptables -A CH_VM_ALLOW -p tcp --dport 53 -j ACCEPT

    # Always allow established/related connections (return traffic)
    iptables -A CH_VM_ALLOW -m state --state RELATED,ESTABLISHED -j ACCEPT

    # Resolve each domain and allow its IPs
    for domain in "${ALLOWED_DOMAINS[@]}"; do
        echo "    Allowing: $domain"
        # Resolve domain to IPs (both A and AAAA, take IPv4 only for iptables)
        ips=$(getent ahostsv4 "$domain" 2>/dev/null | awk '{print $1}' | sort -u || true)
        if [[ -z "$ips" ]]; then
            echo "      WARNING: Could not resolve $domain — skipping"
            continue
        fi
        for ip in $ips; do
            iptables -A CH_VM_ALLOW -d "$ip" -j ACCEPT
            echo "      -> $ip"
        done
    done

    # Drop everything else from the VM
    iptables -A CH_VM_ALLOW -j DROP

    # Hook the chain into FORWARD for VM traffic going outbound
    iptables -C FORWARD -i "$BRIDGE_NAME" ! -o "$BRIDGE_NAME" -j CH_VM_ALLOW 2>/dev/null ||
        iptables -I FORWARD -i "$BRIDGE_NAME" ! -o "$BRIDGE_NAME" -j CH_VM_ALLOW

    echo "  Domain allowlist active (${#ALLOWED_DOMAINS[@]} domains)"
else
    # No allowlist — allow all forwarding from the bridge
    iptables -C FORWARD -i "$BRIDGE_NAME" -j ACCEPT 2>/dev/null ||
        iptables -A FORWARD -i "$BRIDGE_NAME" -j ACCEPT
    iptables -C FORWARD -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null ||
        iptables -A FORWARD -o "$BRIDGE_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT

    echo "  Full internet access (no domain filtering)"
fi

# ── dnsmasq (DHCP server) ───────────────────────────────────────────
# Serves DHCP on the bridge so the VM gets an IP automatically.
# Also acts as DNS forwarder so domain resolution works in the VM.
if [[ -f "$DNSMASQ_PID_FILE" ]] && kill -0 "$(cat "$DNSMASQ_PID_FILE")" 2>/dev/null; then
    echo "  dnsmasq already running (PID $(cat "$DNSMASQ_PID_FILE"))"
else
    dnsmasq \
        --interface="$BRIDGE_NAME" \
        --bind-interfaces \
        --dhcp-range="$DHCP_RANGE_START,$DHCP_RANGE_END,12h" \
        --dhcp-option=3,"$BRIDGE_IP" \
        --dhcp-option=6,"$BRIDGE_IP" \
        --pid-file="$DNSMASQ_PID_FILE" \
        --log-queries \
        --log-facility=/tmp/ch-dnsmasq.log \
        --no-daemon &
    disown
    sleep 0.5
    echo "  dnsmasq started (DHCP: $DHCP_RANGE_START - $DHCP_RANGE_END)"
fi

# ── Output ───────────────────────────────────────────────────────────
# Print the values needed for cloud-hypervisor's --net flag.
echo ""
echo "=== Network ready ==="
echo "  Bridge:    $BRIDGE_NAME ($BRIDGE_IP)"
echo "  TAP:       $TAP_NAME"
echo ""
echo "Use with cloud-hypervisor:"
echo "  --net tap=$TAP_NAME"
echo ""
echo "SSH into VM (after boot):"
echo "  ssh root@<VM_IP>   (password: root)"
echo "  Check VM IP in dnsmasq log: /tmp/ch-dnsmasq.log"
echo ""
echo "Cleanup when done:"
echo "  sudo $0 --cleanup"
