{pkgs}:
# ── Defaults ─────────────────────────────────────────────────────────────────
# These mirror the field names and defaults from the Rust structs:
#   worker/src/config.rs  → Config<F>
#   worker/src/ch/factory.rs → ch::factory::Config
#
# When a Rust field is added, renamed, or removed, update this file first.
# Both apps.nix (dev wrapper) and service.nix (NixOS module) consume mkWorkerConfig,
# so a single change here keeps all callers in sync.
let
  defaults = {
    # worker/src/config.rs
    listenAddr = "0.0.0.0:8080";
    masterAddr = "0.0.0.0:8081";
    healthTickMillis = 1000;

    # worker/src/ch/factory.rs
    vmm = {
      binaryPath = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
      runtimeDir = "/run/procurator-worker";
      stateDir = "/var/lib/procurator-worker";
      bridgeName = "br0";
      # Gateway pushed into every guest via the `procurator.gw=` cmdline token
      # (parsed in stage 2 by the in-image `procurator-netcfg` systemd unit).
      # Must match the IP actually assigned to the bridge on the host
      # (nix/modules/worker/vmm.nix: bridgeAddress).
      bridgeGateway = "10.0.0.1";
      ipPoolStart = "10.0.0.2";
      ipPoolEnd = "10.255.255.254";
      ipNetmask = "255.0.0.0";
    };
  };

  # Builds the JSON attrset the Rust worker binary deserialises on startup.
  # Callers pass only the keys they want to override; the rest come from defaults.
  mkWorkerConfig = {
    listenAddr ? defaults.listenAddr,
    masterAddr ? defaults.masterAddr,
    healthTickMillis ? defaults.healthTickMillis,
    vmm ? {},
  }: let
    resolvedVmm = defaults.vmm // vmm;
  in {
    listen_addr = listenAddr;
    master_addr = masterAddr;
    health_tick_millis = healthTickMillis;
    vmm = {
      binary_path = resolvedVmm.binaryPath;
      runtime_dir = resolvedVmm.runtimeDir;
      state_dir = resolvedVmm.stateDir;
      bridge_name = resolvedVmm.bridgeName;
      bridge_gateway = resolvedVmm.bridgeGateway;
      ip_pool_start = resolvedVmm.ipPoolStart;
      ip_pool_end = resolvedVmm.ipPoolEnd;
      ip_netmask = resolvedVmm.ipNetmask;
    };
  };
in {
  inherit defaults mkWorkerConfig;
}
