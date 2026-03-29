{ config, lib, pkgs, ... }:

with lib;

let
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
    };

    users.groups.${cfg.group} = {};

    systemd.services.procurator-worker = {
      description = "Procurator Worker Node";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/procurator ${configFile}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/procurator-worker" ];
        StateDirectory = "procurator-worker";
      };
    };
  };
}
