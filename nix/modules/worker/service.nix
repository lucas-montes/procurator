{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.procurator.worker;
  clusterCfg = config.cluster.vms or {};

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
      socket_dir = cfg.vmRuntimeDir;
      socket_timeout_secs = cfg.cloudHypervisorSocketTimeoutSeconds;
      bridge_name = cfg.bridgeName;
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
      default = "/run/procurator-worker/vms";
      example = "/run/procurator-worker/vms";
      description = "Directory for per-VM runtime artifacts (sockets, writable disks, logs).";
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
      type = types.nullOr types.str;
      default = "br0";
      example = "br0";
      description = "Bridge name used for VM TAP attachment. Set to null to disable VM networking.";
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
        # /tmp/procurator/vms — per-VM dirs: writable disk copies, serial
        #                       logs, API sockets, CH log files.
        # /run/procurator-worker — RuntimeDirectory for ephemeral state.
        ReadWritePaths = [ cfg.vmRuntimeDir ];
        StateDirectory = "procurator-worker";
        RuntimeDirectory = "procurator-worker";
      };
    };
  };
}
