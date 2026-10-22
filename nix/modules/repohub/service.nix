{
  config,
  lib,
  pkgs,
  ...
}:
with lib; let
  cfg = config.services.procurator.repohub;
  tomlFormat = pkgs.formats.toml { };
  # Why: single TOML file is the server's only config source (no env vars),
  # mirroring `cache.nix`. Keys must match `repohub/src/config.rs`.
  configFile = tomlFormat.generate "procurator-repohub.toml" {
    database_url = cfg.databaseUrl;
    bind_address = cfg.addr;
    domain = cfg.domain;
    repos_base_path = cfg.reposBasePath;
  };
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
      description = "Address and port for the repohub HTTP API to bind to.";
    };

    databaseUrl = mkOption {
      type = types.str;
      default = "sqlite:///var/lib/procurator-repohub/repohub.db";
      example = "postgresql://user:pass@localhost/repohub";
      description = "Database connection URL for the repohub service.";
    };

    domain = mkOption {
      type = types.str;
      default = "homelab";
      description = ''
        Domain used to render SSH URLs for repositories
        (e.g. <user>@<domain>:<user>/<repo>.git).
      '';
    };

    reposBasePath = mkOption {
      type = types.str;
      default = "/var/lib/git-server";
      example = "/var/lib/git-server";
      description = ''
        Where bare git repositories are served from. Defaults to the
        git-server root (services.procurator.gitServer.path). The service
        user needs write access there to create repos (enabled automatically
        when the local git server module is enabled).
      '';
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
      # Why: the repository service creates/mutates bare repos under the
      # git-server root, which is group-writable by `git`.
      extraGroups = let
        gitServerEnabled = config.services.procurator.gitServer.enable or false;
      in (if gitServerEnabled then [ "git" ] else []);
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d /var/lib/procurator-repohub 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.procurator-repohub = {
      description = "Procurator Repohub Git Service";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/repohub --config ${configFile} ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = ["/var/lib/procurator-repohub" cfg.reposBasePath];
        StateDirectory = "procurator-repohub";
      };
    };
  };
}
