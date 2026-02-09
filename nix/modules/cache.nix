{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.cache;
in {
  options.services.procurator.cache = {
    enable = mkEnableOption "Procurator binary cache service";

    package = mkOption {
      type = types.package;
      default = pkgs.cache;
      defaultText = literalExpression "pkgs.cache";
      description = "The cache package to use.";
    };

    addr = mkOption {
      type = types.str;
      default = "0.0.0.0:8081";
      example = "0.0.0.0:8081";
      description = "Address and port for the cache service to bind to.";
    };

    storageDir = mkOption {
      type = types.path;
      default = "/var/lib/procurator-cache";
      description = "Directory for storing cached artifacts.";
    };

    maxSize = mkOption {
      type = types.str;
      default = "50G";
      example = "100G";
      description = "Maximum size for the cache storage (e.g., 50G, 100G).";
    };

    user = mkOption {
      type = types.str;
      default = "procurator-cache";
      description = "User account under which the cache service runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-cache";
      description = "Group under which the cache service runs.";
    };

    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ "--verbose" ];
      description = "Additional command-line arguments to pass to the cache service.";
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator cache service user";
      home = cfg.storageDir;
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d ${cfg.storageDir} 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.procurator-cache = {
      description = "Procurator Binary Cache Service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      environment = {
        CACHE_STORAGE_DIR = cfg.storageDir;
        CACHE_MAX_SIZE = cfg.maxSize;
        CACHE_ADDR = cfg.addr;
      };

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/cache ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.storageDir ];
        StateDirectory = "procurator-cache";
      };
    };
  };
}
