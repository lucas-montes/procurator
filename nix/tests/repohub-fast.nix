# Fast test for the repohub NixOS modules (service + git-server) — pure eval.
#
# Validates:
#   1. Defaults flow into the generated TOML (as JSON via the stub) and
#      ExecStart points at bin/repohub --config
#   2. Repohub service user joins the `git` group when the local git server
#      is enabled (shared reposBasePath)
#   3. git-server creates the git user, hook and config files, with
#      authorized keys and admins parameterized (no hard-coded host users)
#   4. git-server is disabled by default
#
# Run:
#   nix eval --raw -f ./nix/tests/repohub-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows extractExecStartConfig;

  pkgsStub = mkPkgsStub { packages = ["repohub"]; inherit system; };

  evalRepohub = moduleCfg:
    lib.evalModules {
      modules = [
        ../modules/repohub
        stubSystemd
        ({ ... }: moduleCfg)
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  repohubJson = evalResult:
    extractExecStartConfig
      evalResult.config.systemd.services."procurator-repohub".serviceConfig.ExecStart
      "${toString pkgsStub.repohub}/bin/repohub --config ";

in
let
  # ── 1. Service defaults (without the local git server) ─────────────
  defaults = evalRepohub {
    services.procurator.repohub.enable = true;
  };
  defaultsJson = repohubJson defaults;

  defaultsCheck =
    assert' "defaults pass assertions" (checkAssertions defaults)
    && assertEq "bind_address" defaultsJson.bind_address "0.0.0.0:8083"
    && assertEq "database_url" defaultsJson.database_url "sqlite:///var/lib/procurator-repohub/repohub.db"
    && assertEq "domain" defaultsJson.domain "homelab"
    && assertEq "repos_base_path" defaultsJson.repos_base_path "/var/lib/git-server"
    && assert' "git-server off by default" (!(defaults.config ? users.users.git))
    && assert' "no tmpfiles without git-server"
      (builtins.all (r: !(builtins.match ".*git-server.*" r != null)) defaults.config.systemd.tmpfiles.rules);

  # ── 2. Repohub + git-server joined wiring ──────────────────────────
  combined = evalRepohub {
    services.procurator.repohub.enable = true;
    services.procurator.gitServer.enable = true;
    services.procurator.gitServer.admins = [ "lucas" ];
    services.procurator.gitServer.authorizedKeys = [ "ssh-ed25519 AAAA test" ];
    services.procurator.gitServer.ciWebhookUrl = "http://127.0.0.1:8082/api/builds";
  };
  combinedJson = repohubJson combined;

  combinedCheck =
    assert' "combined passes assertions" (checkAssertions combined)
    && assertEq "repos_base_path" combinedJson.repos_base_path "/var/lib/git-server"
    && assertEq "repohub user in git group" combined.config.users.users."procurator-repohub".extraGroups [ "git" ]
    && assert' "git user exists" (combined.config ? users.users.git)
    && assertEq "git user home" combined.config.users.users.git.home "/var/lib/git-server"
    && assertEq "git shell" combined.config.users.users.git.shell "/bin/git-shell"
    && assertEq "git group" combined.config.users.users.git.group "git"
    && assertEq "authorized keys" [ "ssh-ed25519 AAAA test" ] combined.config.users.users.git.openssh.authorizedKeys.keys
    && assertEq "admin joins git group" [ "git" ] combined.config.users.users.lucas.extraGroups
    && assert' "post-receive hook installed" (combined.config ? environment.etc."git-server/post-receive")
    && assert' "gitconfig installed" (combined.config ? environment.etc."git-server/gitconfig")
    && assert' "repo root tmpfile exists"
      (builtins.elem "d /var/lib/git-server 2775 git git - -" combined.config.systemd.tmpfiles.rules)
    && assert' "hooks tmpfile exists"
      (builtins.elem "d /var/lib/git-server/hooks 2775 git git - -" combined.config.systemd.tmpfiles.rules)
    && assert' "hook symlink exists"
      (builtins.elem "L+ /var/lib/git-server/hooks/post-receive - - - - /etc/git-server/post-receive" combined.config.systemd.tmpfiles.rules)
    && assert' "repohub can write repos root"
      (builtins.elem "/var/lib/git-server" combined.config.systemd.services."procurator-repohub".serviceConfig.ReadWritePaths);

  hookSrc = combined.config.environment.etc."git-server/post-receive".source;
  hookCheck =
    assert' "legacy CI port 3000 removed from hook" (builtins.match ".*localhost:3000.*" hookSrc == null)
    && assert' "hook targets configured webhook" (builtins.match ".*127.0.0.1:8082/api/builds.*" hookSrc != null);

  # ── 3. Strict defaults with git-server enabled (no baked-in hosts) ─
  strict = evalRepohub {
    services.procurator.gitServer.enable = true;
  };
  strictCheck =
    assert' "git-server enabled passes assertions" (checkAssertions strict)
    && assertEq "no authorized keys by default" [] strict.config.users.users.git.openssh.authorizedKeys.keys
    && assertEq "no admins by default" [] strict.config.services.procurator.gitServer.admins;

in
defaultsCheck && combinedCheck && hookCheck && strictCheck
