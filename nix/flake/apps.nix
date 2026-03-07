{
  pkgs,
  flake-utils,
  packages,
}: let
  ch = pkgs."cloud-hypervisor";

  inherit (packages)
    cache
    ci_service
    worker
    cli
    ;

  # ── Host network setup script ─────────────────────────────────────
  # One-time sudo setup: bridge, CAP_NET_ADMIN on CH, masquerade, dnsmasq.
  # Idempotent — skips steps already done. Called by the worker wrapper.
  setup-network = pkgs.writeShellScript "pcr-setup-network" ''
    set -euo pipefail
    PATH="${pkgs.lib.makeBinPath (with pkgs; [iproute2 iptables dnsmasq coreutils procps gnugrep libcap])}:$PATH"

    BRIDGE="chbr0"
    BRIDGE_IP="192.168.249.1"
    BRIDGE_SUBNET="192.168.249.0/24"
    DHCP_RANGE_START="192.168.249.10"
    DHCP_RANGE_END="192.168.249.50"
    CH_BIN="${ch}/bin/cloud-hypervisor"
    DNSMASQ_PID="/run/pcr-dnsmasq.pid"
    PCR_LIB="/var/lib/procurator/bin"
    PCR_CH="$PCR_LIB/cloud-hypervisor"

    echo "[pcr-setup] Setting up host networking for procurator worker..."

    # ── 1. Bridge ──────────────────────────────────────────────────
    if ! ip link show "$BRIDGE" &>/dev/null; then
      echo "[pcr-setup] Creating bridge $BRIDGE"
      ip link add "$BRIDGE" type bridge
      ip addr add "$BRIDGE_IP/24" dev "$BRIDGE"
      ip link set "$BRIDGE" up
    else
      echo "[pcr-setup] Bridge $BRIDGE already exists"
    fi

    # ── 2. IP forwarding ───────────────────────────────────────────
    if [ "$(cat /proc/sys/net/ipv4/ip_forward)" != "1" ]; then
      echo "[pcr-setup] Enabling IP forwarding"
      sysctl -w net.ipv4.ip_forward=1
    fi

    # ── 3. Masquerade (NAT for VM outbound traffic) ────────────────
    if ! iptables -t nat -C POSTROUTING -s "$BRIDGE_SUBNET" ! -o "$BRIDGE" -j MASQUERADE 2>/dev/null; then
      echo "[pcr-setup] Adding masquerade rule for $BRIDGE_SUBNET"
      iptables -t nat -A POSTROUTING -s "$BRIDGE_SUBNET" ! -o "$BRIDGE" -j MASQUERADE
    else
      echo "[pcr-setup] Masquerade rule already exists"
    fi

    # Allow forwarding to/from the bridge
    if ! iptables -C FORWARD -i "$BRIDGE" -j ACCEPT 2>/dev/null; then
      iptables -A FORWARD -i "$BRIDGE" -j ACCEPT
    fi
    if ! iptables -C FORWARD -o "$BRIDGE" -j ACCEPT 2>/dev/null; then
      iptables -A FORWARD -o "$BRIDGE" -j ACCEPT
    fi

    # ── 4. CAP_NET_ADMIN on cloud-hypervisor ───────────────────────
    # CH needs this to create TAP devices. We copy to a writable location
    # outside /run because /run is typically mounted nosuid, which makes
    # file capabilities ineffective even if getcap shows them.
    mkdir -p "$PCR_LIB"
    if [ ! -x "$PCR_CH" ] || ! getcap "$PCR_CH" 2>/dev/null | grep -q cap_net_admin; then
      echo "[pcr-setup] Installing cloud-hypervisor with CAP_NET_ADMIN at $PCR_CH"
      cp "$CH_BIN" "$PCR_CH"
      chmod 755 "$PCR_CH"
      setcap cap_net_admin+ep "$PCR_CH"
    else
      echo "[pcr-setup] $PCR_CH already has CAP_NET_ADMIN"
    fi

    # ── 5. dnsmasq (DHCP + DNS for VMs) ────────────────────────────
    if [ -f "$DNSMASQ_PID" ] && kill -0 "$(cat "$DNSMASQ_PID")" 2>/dev/null; then
      echo "[pcr-setup] dnsmasq already running (pid $(cat "$DNSMASQ_PID"))"
    else
      echo "[pcr-setup] Starting dnsmasq on $BRIDGE"
      dnsmasq \
        --interface="$BRIDGE" \
        --bind-interfaces \
        --dhcp-range="$DHCP_RANGE_START,$DHCP_RANGE_END,12h" \
        --dhcp-option="3,$BRIDGE_IP" \
        --dhcp-option="6,$BRIDGE_IP" \
        --except-interface=lo \
        --log-queries \
        --log-dhcp \
        --pid-file="$DNSMASQ_PID" \
        --log-facility=/run/pcr/dnsmasq.log
    fi

    echo "[pcr-setup] Host networking ready."
  '';

  worker-wrapper = pkgs.writeShellScriptBin "procurator-worker" ''
    # ── Host network setup (requires sudo once) ────────────────────
    # Creates bridge, masquerade, dnsmasq, and gives CH CAP_NET_ADMIN.
    # Idempotent — safe to run multiple times.
    if [ -x /run/wrappers/bin/cloud-hypervisor ] && ip link show chbr0 &>/dev/null; then
      # NixOS host module already set up — use its wrapper
      export PATH="/run/wrappers/bin:$PATH"
      export PCR_CH_BINARY="/run/wrappers/bin/cloud-hypervisor"
    elif [ -x /var/lib/procurator/bin/cloud-hypervisor ] && ip link show chbr0 &>/dev/null; then
      # Our setup script already ran — use its binary
      export PATH="/var/lib/procurator/bin:$PATH"
      export PCR_CH_BINARY="/var/lib/procurator/bin/cloud-hypervisor"
    else
      echo "[worker] Host networking not set up. Running one-time setup (requires sudo)..."
      sudo ${setup-network}
      export PATH="/var/lib/procurator/bin:$PATH"
      export PCR_CH_BINARY="/var/lib/procurator/bin/cloud-hypervisor"
    fi

    # ip(8) is needed to attach TAP devices to the bridge
    export PATH="${pkgs.lib.makeBinPath [pkgs.iproute2]}:$PATH"
    exec ${worker}/bin/worker "$@"
  '';

  control-plane-wrapper = pkgs.writeShellScriptBin "procurator-control-plane" ''
    exec ${worker}/bin/worker "$@"
  '';

  pcr-test-wrapper = pkgs.writeShellScriptBin "pcr-test" ''
    exec ${cli}/bin/pcr-test "$@"
  '';
in {
  wrappers = {
    inherit
      worker-wrapper
      control-plane-wrapper
      pcr-test-wrapper
      ;
  };

  apps = {
    cache = flake-utils.lib.mkApp {drv = cache;};
    ci_service = flake-utils.lib.mkApp {drv = ci_service;};
    worker = flake-utils.lib.mkApp {drv = worker-wrapper;};
    pcr-test = flake-utils.lib.mkApp {
      drv = cli;
      exePath = "/bin/pcr-test";
    };
    procurator-worker = flake-utils.lib.mkApp {drv = worker-wrapper;};
    procurator-control-plane = flake-utils.lib.mkApp {drv = control-plane-wrapper;};
    default = flake-utils.lib.mkApp {drv = control-plane-wrapper;};
  };
}
