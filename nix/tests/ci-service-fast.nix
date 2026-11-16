# Fast test for the CI-service NixOS module — pure eval, no NixOS build.
#
# Validates:
#   1. Defaults flow into the generated TOML (as JSON via the stub) and
#      ExecStart points at bin/ci_service --config
#   2. Explicit options override all documented fields
#   3. Service user/group and state dir wiring exists
#
# Run:
#   nix eval --raw -f ./nix/tests/ci-service-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows extractExecStartConfig;

  pkgsStub = mkPkgsStub { packages = ["ci_service" "nix" "nix-eval-jobs"]; inherit system; };

  evalCi = ciCfg:
    lib.evalModules {
      modules = [
        ../modules/ci-service.nix
        stubSystemd
        ({ ... }: { config.services.procurator.ci = ciCfg; })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  ciJson = evalResult:
    extractExecStartConfig
      evalResult.config.systemd.services."procurator-ci".serviceConfig.ExecStart
      "${toString pkgsStub.ci_service}/bin/ci_service --config ";

in
let  # ── 1. Defaults ────────────────────────────────────────────────────
  defaults = evalCi { enable = true; };
  defaultsJson = ciJson defaults;

  defaultsCheck =
    assert' "defaults pass assertions" (checkAssertions defaults)
    && assertEq "bind_address" defaultsJson.bind_address "0.0.0.0:8082"
    && assertEq "database_url" defaultsJson.database_url "sqlite:///var/lib/procurator-ci/ci.db"
    && assertEq "domain" defaultsJson.domain "homelab"
    && assertEq "repos_base_path" defaultsJson.repos_base_path "/var/lib/git-server"
    && assertEq "max_reruns" defaultsJson.max_reruns 3
    && assertEq "max_check_timeout" defaultsJson.max_check_timeout 14400
    && assertEq "max_run_timeout" defaultsJson.max_run_timeout 21600
    && assertEq "worker_poll_interval_ms" defaultsJson.worker_poll_interval_ms 1000
    && assertEq "max_concurrent_runs" defaultsJson.max_concurrent_runs 1
    && assertEq "eval_workers" defaultsJson.eval_workers 1
    && assert' "allowed_uris has github" (builtins.elem "github:" defaultsJson.allowed_uris)
    && assertEq "path has nix, git, nix-eval-jobs"
      defaults.config.systemd.services."procurator-ci".path
      [ pkgsStub.nix pkgsStub.git pkgsStub.nix-eval-jobs ]
    && assertEq "joins the git group"
      defaults.config.users.users."procurator-ci".extraGroups [ "git" ]
    && assert' "no cache by default" (!(defaultsJson ? cache_base_url) && !(defaultsJson ? netrc_file))
    && assert' "no secrets by default" (!(defaultsJson ? secrets_file))
    && assertEq "no substituters by default" defaults.config.nix.settings { };

  # ── 2. Explicit overrides ──────────────────────────────────────────
  explicit = evalCi {
    enable = true;
    addr = "127.0.0.1:9002";
    databaseUrl = "sqlite:///tmp/ci.db";
    domain = "lab";
    reposBasePath = "/srv/git";
    maxReruns = 5;
    maxCheckTimeout = 600;
    maxRunTimeout = 1200;
    workerPollIntervalMillis = 250;
    maxConcurrentRuns = 3;
    evalWorkers = 4;
    allowedUris = [ "git+ssh://git.lab/" ];
    reposGroup = null;
    secretsFile = "/run/secrets/ci-secrets.toml";
    cache = {
      url = "http://cache.lab:8081";
      netrcFile = "/run/secrets/ci-netrc";
      readCaches = [ "private-lucas" ];
      trustedPublicKeys = [ "cache.local:abc=" ];
    };
  };
  explicitJson = ciJson explicit;

  explicitCheck =
    assert' "explicit passes assertions" (checkAssertions explicit)
    && assertEq "bind_address" explicitJson.bind_address "127.0.0.1:9002"
    && assertEq "database_url" explicitJson.database_url "sqlite:///tmp/ci.db"
    && assertEq "domain" explicitJson.domain "lab"
    && assertEq "repos_base_path" explicitJson.repos_base_path "/srv/git"
    && assertEq "max_reruns" explicitJson.max_reruns 5
    && assertEq "max_check_timeout" explicitJson.max_check_timeout 600
    && assertEq "max_run_timeout" explicitJson.max_run_timeout 1200
    && assertEq "worker_poll_interval_ms" explicitJson.worker_poll_interval_ms 250
    && assertEq "max_concurrent_runs" explicitJson.max_concurrent_runs 3
    && assertEq "eval_workers" explicitJson.eval_workers 4
    && assertEq "allowed_uris" explicitJson.allowed_uris [ "git+ssh://git.lab/" ]
    && assertEq "no extra group" explicit.config.users.users."procurator-ci".extraGroups [ ]
    && assertEq "cache_base_url" explicitJson.cache_base_url "http://cache.lab:8081"
    && assertEq "netrc_file" explicitJson.netrc_file "/run/secrets/ci-netrc"
    && assertEq "secrets_file" explicitJson.secrets_file "/run/secrets/ci-secrets.toml"
    && assertEq "public_cache" explicitJson.public_cache "public"
    && assertEq "private_cache_prefix" explicitJson.private_cache_prefix "private-"
    && assertEq "substituters"
      explicit.config.nix.settings.extra-substituters
      [ "http://cache.lab:8081/public" "http://cache.lab:8081/private-lucas" ]
    && assertEq "trusted keys" explicit.config.nix.settings.extra-trusted-public-keys [ "cache.local:abc=" ];

  # ── 3. User/group/workspace wiring ─────────────────────────────────
  wiring = defaults;
  usersCheck =
    assert' "service user exists" (wiring.config ? users.users."procurator-ci")
    && assert' "service group exists" (wiring.config ? users.groups."procurator-ci")
    && assert' "state dir tmpfile rule exists"
      (builtins.elem "d /var/lib/procurator-ci 0750 procurator-ci procurator-ci -" wiring.config.systemd.tmpfiles.rules)
    && assertEq "user in unit" wiring.config.systemd.services."procurator-ci".serviceConfig.User "procurator-ci"
    && assert' "binary is ci_service"
      ((builtins.substring
        0
        (builtins.stringLength "${toString pkgsStub.ci_service}/bin/ci_service --config ")
        (builtins.unsafeDiscardStringContext wiring.config.systemd.services."procurator-ci".serviceConfig.ExecStart))
      == "${toString pkgsStub.ci_service}/bin/ci_service --config ");

in
defaultsCheck && explicitCheck && usersCheck
