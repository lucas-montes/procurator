{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.ci;
  tomlFormat = pkgs.formats.toml { };
  # Why: single TOML file is the server's only config source (no env vars),
  # mirroring `cache.nix`. Keys must match `ci_service/src/config.rs`.
  configFile = tomlFormat.generate "procurator-ci.toml" ({
    database_url = cfg.databaseUrl;
    bind_address = cfg.addr;
    domain = cfg.domain;
    repos_base_path = cfg.reposBasePath;
    max_reruns = cfg.maxReruns;
    max_check_timeout = cfg.maxCheckTimeout;
    max_run_timeout = cfg.maxRunTimeout;
    worker_poll_interval_ms = cfg.workerPollIntervalMillis;
    max_concurrent_runs = cfg.maxConcurrentRuns;
    eval_workers = cfg.evalWorkers;
    allowed_uris = cfg.allowedUris;
    public_cache = cfg.cache.publicCache;
    private_cache_prefix = cfg.cache.privateCachePrefix;
  } // optionalAttrs (cfg.cache.url != null) {
    cache_base_url = cfg.cache.url;
  } // optionalAttrs (cfg.cache.netrcFile != null) {
    netrc_file = cfg.cache.netrcFile;
  });
  cacheUrl = name: "${cfg.cache.url}/${name}";
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

    maxReruns = mkOption {
      type = types.ints.unsigned;
      default = 3;
      description = "Most times a repo's procurator.nix may ask for a failed check to be built again (ci.reruns is capped to this).";
    };

    maxCheckTimeout = mkOption {
      type = types.ints.positive;
      default = 4 * 3600;
      description = "Most seconds a repo may let one derivation build (caps ci.timeouts.check and ci.timeouts.silent).";
    };

    maxRunTimeout = mkOption {
      type = types.ints.positive;
      default = 6 * 3600;
      description = "Most seconds a repo may let one run take (caps ci.timeouts.run).";
    };

    workerPollIntervalMillis = mkOption {
      type = types.ints.positive;
      default = 1000;
      description = "Delay between internal build-queue polls, in milliseconds.";
    };

    maxConcurrentRuns = mkOption {
      type = types.ints.positive;
      default = 1;
      description = "Runs carried out at the same time.";
    };

    evalWorkers = mkOption {
      type = types.ints.positive;
      default = 1;
      description = "nix-eval-jobs worker processes per run (each may use up to 4 GiB).";
    };

    cache = {
      url = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "http://cache.lan:8081";
        description = ''
          Base URL of the Procurator cache (services.procurator.cache); each
          cache is served at <url>/<name>. When set, what a run builds is
          pushed to the repo owner's private cache, or to the public cache
          when the default branch's procurator.nix sets ci.cache to it.
          null: nothing is pushed.
        '';
      };

      publicCache = mkOption {
        type = types.str;
        default = "public";
        description = "Name of the public cache.";
      };

      privateCachePrefix = mkOption {
        type = types.str;
        default = "private-";
        description = "Each owner's private cache is <prefix><owner>; create one per owner on the cache.";
      };

      netrcFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "/run/secrets/procurator-ci-netrc";
        description = ''
          Nix netrc file with the CI's cache token
          (`machine <cache host> password pcr_…`), readable by the service
          user only. The token needs `write` on the public cache and on every
          owner's private cache. Keep it out of the Nix store.
        '';
      };

      substitute = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Add the public cache and readCaches to the host's substituters, so
          evaluation sees what users already pushed and nothing is rebuilt.
        '';
      };

      readCaches = mkOption {
        type = types.listOf types.str;
        default = [ ];
        example = [ "private-lucas" ];
        description = ''
          Private caches the host's nix-daemon substitutes from. The daemon
          reads them with nix.settings.netrc-file, which you set yourself.
        '';
      };

      trustedPublicKeys = mkOption {
        type = types.listOf types.str;
        default = [ ];
        example = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
        description = "Signing keys of the caches above (the cache prints them at startup).";
      };
    };

    allowedUris = mkOption {
      type = types.listOf types.str;
      default = [ "github:" "gitlab:" "https://github.com/" "https://gitlab.com/" ];
      description = ''
        URI prefixes a pushed flake may fetch inputs from during evaluation
        (Nix `allowed-uris`, with `restrict-eval` on). Add your own forges here.
      '';
    };

    reposGroup = mkOption {
      type = types.nullOr types.str;
      default = "git";
      description = ''
        Group that can read the bare repositories under reposBasePath (the
        git-server's `git` group). The service user joins it. null to manage
        access yourself.
      '';
    };

    nixEvalJobsPackage = mkOption {
      type = types.package;
      default = pkgs.nix-eval-jobs;
      defaultText = literalExpression "pkgs.nix-eval-jobs";
      description = "nix-eval-jobs, used to evaluate checks in parallel.";
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
    # Why: a check users already pushed is then `cached` at evaluation and
    # never built here.
    nix.settings = mkIf (cfg.cache.url != null && cfg.cache.substitute) {
      extra-substituters = map cacheUrl ([ cfg.cache.publicCache ] ++ cfg.cache.readCaches);
      extra-trusted-public-keys = cfg.cache.trustedPublicKeys;
    };

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator CI service user";
      home = "/var/lib/procurator-ci";
      extraGroups = optional (cfg.reposGroup != null) cfg.reposGroup;
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d /var/lib/procurator-ci 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.procurator-ci = {
      description = "Procurator CI/CD Service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      # Why: the worker runs nix-eval-jobs, `nix build` and `nix-store`; Nix
      # fetches the commit from the bare repo with git. Builds go through
      # the daemon, which sends them to the host's remote builders
      # (nix.buildMachines) and builds locally otherwise. The service user
      # must not be in nix.settings.trusted-users.
      path = [ (config.nix.package or pkgs.nix) pkgs.git cfg.nixEvalJobsPackage ];

      environment = {
        # Why: the repos belong to `git`; git refuses repos owned by
        # another user unless they are marked safe.
        GIT_CONFIG_COUNT = "1";
        GIT_CONFIG_KEY_0 = "safe.directory";
        GIT_CONFIG_VALUE_0 = "*";
      };

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/ci_service --config ${configFile} ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Why: CI writes only its DB/state dir (also its HOME, where Nix
        # keeps its fetcher cache). The repos root is read-only (git-server
        # owns it); protect everything else.
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
