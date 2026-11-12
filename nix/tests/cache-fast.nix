# Fast test for the cache NixOS service module — pure eval, no NixOS build.
#
# Validates:
#   1. Defaults flow into the generated TOML (as JSON via the stub) and
#      ExecStart points at bin/cache --config
#   2. Named caches, GC, and the admin token hash reach the generated config
#   3. At least one cache must be defined (assertion)
#   4. The admin token hash must be 64 hex characters (assertion)
#
# Run:
#   nix eval -f ./nix/tests/cache-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows extractExecStartConfig;

  pkgsStub = mkPkgsStub { packages = ["cache"]; inherit system; };

  evalCache = cacheCfg:
    lib.evalModules {
      modules = [
        ../modules/cache.nix
        stubSystemd
        ({ ... }: { config.services.procurator.cache = cacheCfg; })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  cacheJson = evalResult:
    extractExecStartConfig
      evalResult.config.systemd.services."procurator-cache".serviceConfig.ExecStart
      "${toString pkgsStub.cache}/bin/cache --config ";

  hash = "1734d503f6aa6a047c36d113cbad769f719c93784b469b771c4c3e7c63adbefd";
in
let
  # ── 1. Defaults ────────────────────────────────────────────────────
  defaults = evalCache {
    enable = true;
    caches.public = { visibility = "public"; signKeyFile = "/run/secrets/public.key"; };
  };
  defaultsJson = cacheJson defaults;

  defaultsCheck =
    assert' "defaults pass assertions" (checkAssertions defaults)
    && assertEq "addr" defaultsJson.addr "0.0.0.0:8081"
    && assertEq "storage_dir" defaultsJson.storage_dir "/var/lib/procurator-cache"
    && assertEq "gc" defaultsJson.gc { interval_secs = 3600; orphan_grace_secs = 86400; }
    && assert' "no admin token by default" (!(defaultsJson ? admin_token_sha256))
    && assertEq "caches.public" defaultsJson.caches.public {
      kind = "nix";
      visibility = "public";
      sign_key_file = "/run/secrets/public.key";
      store_dir = "/nix/store";
      priority = 30;
      max_size = "0";
      retention_days = 0;
    };

  # ── 2. Explicit options flow through ───────────────────────────────
  explicit = evalCache {
    enable = true;
    addr = "127.0.0.1:9001";
    storageDir = "/srv/cache";
    adminTokenSha256 = hash;
    gc.intervalSecs = 600;
    gc.orphanGraceSecs = 120;
    caches.public = { visibility = "public"; signKeyFile = "/run/secrets/public.key"; };
    caches.internal = {
      visibility = "private";
      signKeyFile = "/run/secrets/internal.key";
      priority = 10;
      maxSize = "200G";
      retentionDays = 90;
    };
  };
  explicitJson = cacheJson explicit;

  explicitCheck =
    assert' "explicit passes assertions" (checkAssertions explicit)
    && assertEq "addr" explicitJson.addr "127.0.0.1:9001"
    && assertEq "storage_dir" explicitJson.storage_dir "/srv/cache"
    && assertEq "admin_token_sha256" explicitJson.admin_token_sha256 hash
    && assertEq "gc" explicitJson.gc { interval_secs = 600; orphan_grace_secs = 120; }
    && assertEq "cache names" (builtins.attrNames explicitJson.caches) [ "internal" "public" ]
    && assertEq "internal.visibility" explicitJson.caches.internal.visibility "private"
    && assertEq "internal.sign_key_file" explicitJson.caches.internal.sign_key_file "/run/secrets/internal.key"
    && assertEq "internal.priority" explicitJson.caches.internal.priority 10
    && assertEq "internal.max_size" explicitJson.caches.internal.max_size "200G"
    && assertEq "internal.retention_days" explicitJson.caches.internal.retention_days 90;

  # ── 3. At least one cache ──────────────────────────────────────────
  noCaches = assertThrows "no caches defined"
    (checkAssertions (evalCache { enable = true; }));

  # ── 4. Admin token hash shape ──────────────────────────────────────
  badHash = assertThrows "malformed admin token hash"
    (checkAssertions (evalCache {
      enable = true;
      adminTokenSha256 = "not-a-hash";
      caches.public = { visibility = "public"; signKeyFile = "/run/secrets/public.key"; };
    }));

in
defaultsCheck && explicitCheck && noCaches && badHash
