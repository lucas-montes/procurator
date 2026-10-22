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

  pkgsStub = mkPkgsStub { packages = ["ci_service"]; inherit system; };

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
    && assertEq "max_retries" defaultsJson.max_retries 3
    && assertEq "worker_poll_interval_ms" defaultsJson.worker_poll_interval_ms 1000;

  # ── 2. Explicit overrides ──────────────────────────────────────────
  explicit = evalCi {
    enable = true;
    addr = "127.0.0.1:9002";
    databaseUrl = "sqlite:///tmp/ci.db";
    domain = "lab";
    reposBasePath = "/srv/git";
    maxRetries = 5;
    workerPollIntervalMillis = 250;
  };
  explicitJson = ciJson explicit;

  explicitCheck =
    assert' "explicit passes assertions" (checkAssertions explicit)
    && assertEq "bind_address" explicitJson.bind_address "127.0.0.1:9002"
    && assertEq "database_url" explicitJson.database_url "sqlite:///tmp/ci.db"
    && assertEq "domain" explicitJson.domain "lab"
    && assertEq "repos_base_path" explicitJson.repos_base_path "/srv/git"
    && assertEq "max_retries" explicitJson.max_retries 5
    && assertEq "worker_poll_interval_ms" explicitJson.worker_poll_interval_ms 250;

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
