{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.ci;
in {
  options.services.procurator.ci = {
    enable = mkEnableOption "Procurator CI/CD service";

    package = mkOption {
      type = types.package;
      default = pkgs.ci_service;
      defaultText = literalExpression "pkgs.ci_service";
      description = "The ci_service package to use.";
    };

    addr = mkOption {
      type = types.str;
      default = "0.0.0.0:8082";
      example = "0.0.0.0:8082";
      description = "Address and port for the CI service to bind to.";
    };

    databaseUrl = mkOption {
      type = types.str;
      default = "sqlite:///var/lib/procurator-ci/ci.db";
      example = "postgresql://user:pass@localhost/ci";
      description = "Database connection URL for the CI service.";
    };

    workDir = mkOption {
      type = types.path;
      default = "/var/lib/procurator-ci/builds";
      description = "Working directory for build operations.";
    };

    cacheUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "http://localhost:8081";
      description = "URL of the binary cache service. If null, cache integration is disabled.";
    };

    maxConcurrentBuilds = mkOption {
      type = types.int;
      default = 4;
      description = "Maximum number of concurrent build jobs.";
    };

    user = mkOption {
      type = types.str;
      default = "procurator-ci";
      description = "User account under which the CI service runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-ci";
      description = "Group under which the CI service runs.";
    };

    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional command-line arguments.";
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator CI service user";
      home = "/var/lib/procurator-ci";
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d /var/lib/procurator-ci 0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.workDir} 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.procurator-ci = {
      description = "Procurator CI/CD Service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      environment = {
        CI_ADDR = cfg.addr;
        CI_DATABASE_URL = cfg.databaseUrl;
        CI_WORK_DIR = cfg.workDir;
        CI_MAX_CONCURRENT_BUILDS = toString cfg.maxConcurrentBuilds;
      } // optionalAttrs (cfg.cacheUrl != null) {
        CI_CACHE_URL = cfg.cacheUrl;
      };

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/ci_service ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/procurator-ci" cfg.workDir ];
        StateDirectory = "procurator-ci";
      };
    };
  };
}
