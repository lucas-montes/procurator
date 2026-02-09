{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.repohub;
in {
  options.services.procurator.repohub = {
    enable = mkEnableOption "Procurator Repohub git service";

    package = mkOption {
      type = types.package;
      default = pkgs.repohub;
      defaultText = literalExpression "pkgs.repohub";
      description = "The repohub package to use.";
    };

    addr = mkOption {
      type = types.str;
      default = "0.0.0.0:8083";
      example = "0.0.0.0:8083";
      description = "Address and port for the repohub service to bind to.";
    };

    repositoriesDir = mkOption {
      type = types.path;
      default = "/var/lib/procurator-repohub/repos";
      description = "Directory for storing git repositories.";
    };

    databaseUrl = mkOption {
      type = types.str;
      default = "sqlite:///var/lib/procurator-repohub/repohub.db";
      example = "postgresql://user:pass@localhost/repohub";
      description = "Database connection URL for the repohub service.";
    };

    webhookUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "http://localhost:8082/webhook";
      description = "URL to send webhook notifications (e.g., to CI service). If null, webhooks are disabled.";
    };

    user = mkOption {
      type = types.str;
      default = "procurator-repohub";
      description = "User account under which the repohub service runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-repohub";
      description = "Group under which the repohub service runs.";
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
      description = "Procurator repohub service user";
      home = "/var/lib/procurator-repohub";
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d /var/lib/procurator-repohub 0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.repositoriesDir} 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.procurator-repohub = {
      description = "Procurator Repohub Git Service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      environment = {
        REPOHUB_ADDR = cfg.addr;
        REPOHUB_REPOS_DIR = cfg.repositoriesDir;
        REPOHUB_DATABASE_URL = cfg.databaseUrl;
      } // optionalAttrs (cfg.webhookUrl != null) {
        REPOHUB_WEBHOOK_URL = cfg.webhookUrl;
      };

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/repohub ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/procurator-repohub" cfg.repositoriesDir ];
        StateDirectory = "procurator-repohub";
      };
    };
  };
}
