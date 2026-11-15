# Fast test for the console NixOS module — pure eval, no NixOS build.
#
# Validates:
#   1. Cluster-referenced control plane and workers become journal-gatewayd
#      sources (host from deployment.addr, gateway port, unit names)
#   2. Local and extra sources render, with http_addr and tuning knobs
#   3. Unknown cluster names and an empty source list fail assertions
#
# Run:
#   nix eval --raw -f ./nix/tests/console-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows extractExecStartConfig;

  pkgsStub = mkPkgsStub { packages = ["console"]; inherit system; } // { systemd = "/sys"; };

  evalConsole = consoleCfg: clusterVms:
    lib.evalModules {
      modules = [
        ../modules/console.nix
        ../modules/cluster.nix
        stubSystemd
        ({ ... }: {
          config.services.procurator.console = consoleCfg;
          config.cluster.vms = clusterVms;
        })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  consoleConfig = evalResult:
    extractExecStartConfig
      evalResult.config.systemd.services."procurator-console".serviceConfig.ExecStart
      "${toString pkgsStub.console}/bin/console ";

  vm = role: addr: { inherit role; deployment = { inherit addr; }; };
  cluster = {
    cp-1 = vm "control-plane" "10.0.0.10";
    worker-1 = vm "worker" "10.0.0.11:9080";
  };
in
let
  # ── 1. Cluster-derived gateway sources ─────────────────────────────
  clustered = evalConsole
    { enable = true; controlPlane = "cp-1"; workers = [ "worker-1" ]; }
    cluster;
  clusteredJson = consoleConfig clustered;

  clusterCheck =
    assert' "clustered passes assertions" (checkAssertions clustered)
    && assertEq "default http_addr" clusteredJson.http_addr "127.0.0.1:8095"
    && assertEq "gateway sources" clusteredJson.sources [
      {
        name = "control-plane";
        service = "control_plane";
        kind.journal_gateway = { url = "http://10.0.0.10:19531"; unit = "procurator-control-plane.service"; };
      }
      {
        name = "worker-1";
        service = "worker:worker-1";
        kind.journal_gateway = { url = "http://10.0.0.11:19531"; unit = "procurator-worker.service"; };
      }
    ];

  # ── 2. Local + extra sources and knobs ─────────────────────────────
  local = evalConsole
    {
      enable = true;
      httpAddr = "0.0.0.0:9000";
      bootstrapEntries = 50;
      staleAfterMillis = 3000;
      local.controlPlane = true;
      local.workers = [ "w-local" ];
      extraSources = [ { name = "file-cp"; service = "control_plane"; kind.file.path = "/var/log/cp.log"; } ];
    }
    { };
  localJson = consoleConfig local;

  localCheck =
    assert' "local passes assertions" (checkAssertions local)
    && assertEq "http_addr" localJson.http_addr "0.0.0.0:9000"
    && assertEq "bootstrap_entries" localJson.bootstrap_entries 50
    && assertEq "stale_after_millis" localJson.stale_after_millis 3000
    && assertEq "local + extra sources" localJson.sources [
      { name = "control-plane"; service = "control_plane"; kind.journal_local.unit = "procurator-control-plane.service"; }
      { name = "w-local"; service = "worker:w-local"; kind.journal_local.unit = "procurator-worker.service"; }
      { name = "file-cp"; service = "control_plane"; kind.file.path = "/var/log/cp.log"; }
    ];

  # ── 3. Fail fast ───────────────────────────────────────────────────
  unknownVm = assertThrows "unknown cluster name"
    (checkAssertions (evalConsole { enable = true; workers = [ "nope" ]; } cluster));
  noSources = assertThrows "no sources"
    (checkAssertions (evalConsole { enable = true; } { }));
in
clusterCheck && localCheck && unknownVm && noSources
