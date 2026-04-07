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
    hostname = cfg.hostname;
    addr = cfg.addr;
    role = {
      Worker = {
        master_addr = derivedMasterAddr;
      };
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

    hostname = mkOption {
      type = types.str;
      default = config.networking.hostName;
      defaultText = literalExpression "config.networking.hostName";
      description = "Hostname for this worker node.";
    };

    addr = mkOption {
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
  };

  config = mkIf cfg.enable {
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
        ReadWritePaths = [ "/tmp/procurator/vms" ];
        StateDirectory = "procurator-worker";
        RuntimeDirectory = "procurator-worker";
      };
    };
  };
}
