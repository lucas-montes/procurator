{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.procurator.worker;
  clusterCfg = config.cluster.vms or {};

  # Derive short names for systemd RuntimeDirectory/StateDirectory from
  # the configured absolute paths so we can keep systemd-managed names in
  # sync with the module options. We prefer the top-level directory name
  # under /run and /var/lib respectively.
  runtimeDirName =
    let parts = builtins.splitString "/" cfg.vmRuntimeDir; in
    # parts = ["" "run" "procurator-worker" "vms" ...]
    builtins.elemAt parts 2;

  stateDirName = builtins.baseNameOf cfg.vmStateDir;

  # Derive master address from cluster config if using cluster-based setup
  derivedMasterAddr =
    if cfg.master != null
    then clusterCfg.${cfg.master}.deployment.addr
    else cfg.masterAddr;

  configFile = pkgs.writeText "procurator-worker-config.json" (builtins.toJSON {
    listen_addr = cfg.listenAddr;
    master_addr = derivedMasterAddr;
    cloud_hypervisor = {
      binary_path = cfg.cloudHypervisorBinaryPath;
      runtime_dir = cfg.vmRuntimeDir;
      state_dir = cfg.vmStateDir;
      socket_timeout_secs = cfg.cloudHypervisorSocketTimeoutSeconds;
      bridge_name = cfg.bridgeName;
      ip_pool_start = cfg.ipPoolStart;
      ip_pool_end = cfg.ipPoolEnd;
      ip_netmask = cfg.ipNetmask;
    };
  });
in {
  options.services.procurator.worker = {
    enable = mkEnableOption "Procurator worker service";

    package = mkOption {
      type = types.package;
      default = pkgs.procurator;
      defaultText = literalExpression "pkgs.procurator";
      description = "The procurator package to use.";
    };

    listenAddr = mkOption {
      type = types.str;
      example = "0.0.0.0:8080";
      description = "Address and port for the worker to bind to.";
    };

    master = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "control-plane-1";
      description = ''
        VM name from cluster.vms to use as the master control plane.
        Its deployment.addr will be used automatically.
        Takes precedence over masterAddr if both are set.
      '';
    };

    masterAddr = mkOption {
      type = types.str;
      default = "";
      example = "192.168.1.10:8080";
      description = ''
        Direct address and port of the control plane master.
        Only used if master is null.
      '';
    };

    user = mkOption {
      type = types.str;
      default = "procurator-worker";
      description = "User account under which the worker runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-worker";
      description = "Group under which the worker runs.";
    };

    vmRuntimeDir = mkOption {
      type = types.str;
      # Keep the runtime directory as the systemd RuntimeDirectory root.
      # Per-VM subdirs are created by the worker underneath this path.
      default = "/run/procurator-worker";
      example = "/run/procurator-worker";
      description = "Base directory for per-VM ephemeral runtime artifacts (sockets, writable disks, logs).";
    };

    vmStateDir = mkOption {
      type = types.str;
      default = "/var/lib/procurator-worker";
      example = "/var/lib/procurator-worker";
      description = "Directory for persistent state (images cache, logs that survive reboots).";
    };

    cloudHypervisorBinaryPath = mkOption {
      type = types.str;
      default = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
      defaultText = literalExpression "\"${pkgs.cloud-hypervisor}/bin/cloud-hypervisor\"";
      description = "Absolute path to the cloud-hypervisor binary used by the worker.";
    };

    cloudHypervisorSocketTimeoutSeconds = mkOption {
      type = types.ints.positive;
      default = 10;
      example = 5;
      description = "Max seconds to wait for cloud-hypervisor API socket creation.";
    };

    bridgeName = mkOption {
      type = types.str;
      default = "br0";
      example = "br0";
      description = "Bridge name used for VM TAP attachment.";
    };

    ipPoolStart = mkOption {
      type = types.str;
      default = "10.0.0.2";
      description = "First IP the worker will assign to a VM. Must be inside the bridge subnet.";
    };

    ipPoolEnd = mkOption {
      type = types.str;
      default = "10.255.255.254";
      description = "Last IP in the pool. 10.0.0.2-10.255.255.254 gives ~16 million addresses.";
    };

    ipNetmask = mkOption {
      type = types.str;
      default = "255.0.0.0";
      description = "Subnet mask corresponding to the bridge prefix. Must match vmm.bridgePrefixLength.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = (cfg.master != null) || (cfg.masterAddr != "");
        message = ''
          services.procurator.worker: you must set either `master` or `masterAddr`.
          - Use `master` to reference a VM from `cluster.vms`.
          - Use `masterAddr` for a direct control-plane address.
        '';
      }
      {
        assertion = (cfg.master == null) || (builtins.hasAttr cfg.master clusterCfg);
        message = "services.procurator.worker: `master` is set but not found in `cluster.vms`.";
      }
    ];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator worker daemon user";
      createHome = false;
      shell = pkgs.runtimeShell;
      # kvm       → /dev/kvm access for hardware-accelerated virtualisation
      # netdev    → /dev/net/tun access for TAP device creation (ioctl TUNSETIFF)
      # The "network" group is not a standard NixOS group; replaced by real device groups.
      extraGroups = [ "kvm" "netdev" ];
    };

    users.groups.${cfg.group} = {};

    systemd.services.procurator-worker = {
      description = "Procurator Worker Node";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;

        # ── Supplementary groups ──────────────────────────────────────
        # Needed so the service (and child processes like cloud-hypervisor)
        # can open /dev/kvm and /dev/net/tun without root.
        SupplementaryGroups = [ "kvm" "netdev" ];

        ExecStart = "${cfg.package}/bin/procurator ${configFile}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Ensure runtime and state directories exist and are owned by the
        # configured service user. This is a safety-net for systems without
        # tmpfiles.d/systemd RuntimeDirectory in use (e.g. during manual
        # testing). ExecStartPre runs before the main process is started.
        ExecStartPre = [
          "${pkgs.coreutils}/bin/mkdir -p ${cfg.vmRuntimeDir}"
          "${pkgs.coreutils}/bin/mkdir -p ${cfg.vmStateDir}"
          "${pkgs.coreutils}/bin/chown -R ${cfg.user}:${cfg.group} ${cfg.vmRuntimeDir}"
          "${pkgs.coreutils}/bin/chown -R ${cfg.user}:${cfg.group} ${cfg.vmStateDir}"
        ];

        # ── Capabilities ──────────────────────────────────────────────
        # CAP_NET_ADMIN — create/delete TAP devices, attach to bridges,
        #                 set link up/down via netlink.
        # CAP_NET_RAW   — needed by CH for raw packet I/O on virtio-net.
        #
        # Ambient caps are inherited by child processes (cloud-hypervisor)
        # even with NoNewPrivileges=true. This is the correct mechanism:
        # ambient caps survive fork+exec without requiring setuid or
        # file capabilities.
        AmbientCapabilities = [ "CAP_NET_ADMIN" "CAP_NET_RAW" ];
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" "CAP_NET_RAW" ];

        # ── Device access ─────────────────────────────────────────────
        # Explicit allowlist prevents future hardening (PrivateDevices)
        # from accidentally blocking required devices.
        #   /dev/net/tun — TAP creation via ioctl (worker creates TAPs)
        #   /dev/kvm     — hardware virtualisation (child CH processes)
        #   /dev/urandom — entropy source configured in CH's rng.src
        #   /dev/vhost-net — optional; CH uses it for vhost-net acceleration
        DevicePolicy = "closed";
        DeviceAllow = [
          "/dev/net/tun rw"
          "/dev/kvm rw"
          "/dev/urandom r"
          "/dev/vhost-net rw"
        ];

        # ── Security hardening ────────────────────────────────────────
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;

        # ── Writable paths ────────────────────────────────────────────
        # `cfg.vmRuntimeDir` is the absolute path where the worker places
        # per-VM runtime artifacts (cloud-hypervisor sockets, writable disk
        # copies, logs). Systemd has several helpers and hardening options
        # that interact with these settings — below is an explanation:
        #
        # - `RuntimeDirectory = "procurator-worker"`
        #     Instructs systemd to create `/run/procurator-worker` before the
        #     service starts and remove it after the service stops. It is
        #     intended for ephemeral runtime state (sockets, pidfiles).
        #     The created directory is owned by the unit `User`/`Group` and
        #     therefore writable by the service.
        #
        # - `StateDirectory = "procurator-worker"`
        #     Instructs systemd to create `/var/lib/procurator-worker` for
        #     state that should persist across reboots. Like RuntimeDirectory,
        #     ownership is set to the unit `User`/`Group`.
        #
        # - `ReadWritePaths`
        #     When `ProtectSystem`, `ProtectHome`, or similar hardening are in
        #     effect, the unit is denied write access except for paths listed
        #     in `ReadWritePaths`. You must include every absolute path the
        #     service (and its children) needs to write. If `vmRuntimeDir` is
        #     a nested path such as `/run/procurator-worker/vms`, include both
        #     the parent runtime directory and the nested path here so writes
        #     are permitted.
        #
        # Recommendation: keep `vmRuntimeDir = "/run/procurator-worker"` so
        # RuntimeDirectory/StateDirectory align with the configured runtime
        # paths. If you prefer a subdirectory (e.g. `/run/procurator-worker/vms`)
        # ensure `ReadWritePaths` includes `/run/procurator-worker` and the
        # subpath in addition to `/var/lib/procurator-worker`.
        # Ensure systemd's named directories match the paths the service
        # actually uses: derive the short names above and set them here so
        # they cannot accidentally diverge from the options the service
        # consumes (cfg.vmRuntimeDir / cfg.vmStateDir).
        ReadWritePaths = [ cfg.vmRuntimeDir cfg.vmStateDir ];
        StateDirectory = stateDirName;
        RuntimeDirectory = runtimeDirName;
      };
    };
  };
}
