# Fast test for the cache NixOS service module — pure eval, no NixOS build.
#
# Validates:
#   1. Defaults flow into the generated TOML (as JSON via the stub) and
#      ExecStart points at bin/cache --config
#   2. TLS + access + signing key options reach the generated config
#   3. certFile/keyFile must be set together (assertion)
#   4. mTLS clientCa requires certFile/keyFile (assertion)
#
# Run:
#   nix eval --raw -f ./nix/tests/cache-fast.nix --apply "f: f {}"
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

in
let
  # ── 1. Defaults ────────────────────────────────────────────────────
  defaults = evalCache { enable = true; };
  defaultsJson = cacheJson defaults;

  defaultsCheck =
    assert' "defaults pass assertions" (checkAssertions defaults)
    && assertEq "addr" defaultsJson.addr "0.0.0.0:8081"
    && assertEq "store_dir" defaultsJson.store_dir "/nix/store"
    && assertEq "storage_dir" defaultsJson.storage_dir "/var/lib/procurator-cache"
    && assertEq "max_size" defaultsJson.max_size "50G"
    && assertEq "priority" defaultsJson.priority 30
    && assertEq "tls" defaultsJson.tls { provider = "file"; writer_allowlist = []; }
    && assertEq "access" defaultsJson.access {
      reads_require_auth = false;
      writes_require_auth = "auto";
      loopback_bypass = true;
    };

  # ── 2. Explicit TLS/access/signing options flow through ────────────
  explicit = evalCache {
    enable = true;
    addr = "127.0.0.1:9001";
    storageDir = "/srv/cache";
    maxSize = "10G";
    priority = 40;
    secretKeyFiles = [ "/run/secrets/cache.key" ];
    tls.certFile = "/run/secrets/cert.pem";
    tls.keyFile = "/run/secrets/key.pem";
    tls.clientCaFile = "/run/secrets/ca.pem";
    tls.writerAllowlist = [ "ci-writer" ];
    access.readsRequireAuth = true;
    access.writesRequireAuth = "always";
    access.loopbackBypass = false;
  };
  explicitJson = cacheJson explicit;

  explicitCheck =
    assert' "explicit passes assertions" (checkAssertions explicit)
    && assertEq "addr" explicitJson.addr "127.0.0.1:9001"
    && assertEq "storage_dir" explicitJson.storage_dir "/srv/cache"
    && assertEq "max_size" explicitJson.max_size "10G"
    && assertEq "priority" explicitJson.priority 40
    && assertEq "sign_key_files" explicitJson.sign_key_files [ "/run/secrets/cache.key" ]
    && assertEq "tls.cert_file" explicitJson.tls.cert_file "/run/secrets/cert.pem"
    && assertEq "tls.key_file" explicitJson.tls.key_file "/run/secrets/key.pem"
    && assertEq "tls.client_ca_file" explicitJson.tls.client_ca_file "/run/secrets/ca.pem"
    && assertEq "tls.writer_allowlist" explicitJson.tls.writer_allowlist [ "ci-writer" ]
    && assertEq "access" explicitJson.access {
      reads_require_auth = true;
      writes_require_auth = "always";
      loopback_bypass = false;
    };

  # ── 3. certFile/keyFile must be set together ───────────────────────
  certOnly = assertThrows "certFile without keyFile"
    (checkAssertions (evalCache {
      enable = true;
      tls.certFile = "/run/secrets/cert.pem";
    }));

  # ── 4. clientCa requires certFile/keyFile ──────────────────────────
  clientCaOnly = assertThrows "clientCaFile without pair"
    (checkAssertions (evalCache {
      enable = true;
      tls.clientCaFile = "/run/secrets/ca.pem";
    }));

in
defaultsCheck && explicitCheck && certOnly && clientCaOnly
