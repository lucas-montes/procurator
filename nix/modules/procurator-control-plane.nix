{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.control-plane;
  clusterCfg = config.cluster.vms or {};

  # Derive peers from cluster config if using cluster-based setup
  derivedPeers =
    if cfg.peers != []
    then map (vm: clusterCfg.${vm}.deployment.addr) cfg.peers
    else cfg.peersAddr;

  configFile = pkgs.writeText "procurator-control-plane-config.json" (builtins.toJSON {
    hostname = cfg.hostname;
    addr = cfg.addr;
    role = {
      Master = {
        peers_addr = derivedPeers;
      };
    };
  });
in {
  options.services.procurator.control-plane = {
    enable = mkEnableOption "Procurator control plane service";

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
      description = "Hostname for this control plane node.";
    };

    addr = mkOption {
      type = types.str;
      example = "0.0.0.0:8080";
      description = "Address and port for the control plane to bind to.";
    };

    peers = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ "worker-1" "worker-2" ];
      description = ''
        List of VM names from cluster.vms to use as peers.
        Their deployment.addr will be used automatically.
        Takes precedence over peersAddr if both are set.
      '';
    };

    peersAddr = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ "192.168.1.11:8080" "192.168.1.12:8080" ];
      description = ''
        Direct list of peer control plane addresses for HA setup.
        Only used if peers is empty.
      '';
    };

    user = mkOption {
      type = types.str;
      default = "procurator-control-plane";
      description = "User account under which the control plane runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-control-plane";
      description = "Group under which the control plane runs.";
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator control plane daemon user";
    };

    users.groups.${cfg.group} = {};

    systemd.services.procurator-control-plane = {
      description = "Procurator Control Plane";
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
        ReadWritePaths = [ "/var/lib/procurator-control-plane" ];
        StateDirectory = "procurator-control-plane";
      };
    };
  };
}
