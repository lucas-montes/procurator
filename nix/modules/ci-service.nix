{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.ci;
  tomlFormat = pkgs.formats.toml { };
  # Why: single TOML file is the server's only config source (no env vars),
  # mirroring `cache.nix`. Keys must match `ci_service/src/config.rs`.
  configFile = tomlFormat.generate "procurator-ci.toml" {
    database_url = cfg.databaseUrl;
    bind_address = cfg.addr;
    domain = cfg.domain;
    repos_base_path = cfg.reposBasePath;
    max_retries = cfg.maxRetries;
    worker_poll_interval_ms = cfg.workerPollIntervalMillis;
  };
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
      description = "Address and port for the CI service HTTP API to bind to.";
    };

    databaseUrl = mkOption {
      type = types.str;
      default = "sqlite:///var/lib/procurator-ci/ci.db";
      example = "postgresql://user:pass@localhost/ci";
      description = "Database connection URL for the CI service.";
    };

    domain = mkOption {
      type = types.str;
      default = "homelab";
      description = "Domain used when rendering external links.";
    };

    reposBasePath = mkOption {
      type = types.str;
      default = "/var/lib/git-server";
      example = "/var/lib/git-server";
      description = ''
        Where bare git repositories are served from (the git-server root, see
        services.procurator.gitServer). Must exist and be readable by the
        service user.
      '';
    };

    maxRetries = mkOption {
      type = types.int;
      default = 3;
      description = "Retry budget for failed build jobs.";
    };

    workerPollIntervalMillis = mkOption {
      type = types.ints.positive;
      default = 1000;
      description = "Delay between internal build-queue polls, in milliseconds.";
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
    ];

    systemd.services.procurator-ci = {
      description = "Procurator CI/CD Service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/ci_service --config ${configFile} ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Why: CI writes only its DB/state dir. The repos root is read-only
        # (git-server owns it); protect everything else.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/procurator-ci" ];
        StateDirectory = "procurator-ci";
      };
    };
  };
}
