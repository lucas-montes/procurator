# Fast test for the control-plane NixOS module — pure eval, no NixOS build.
#
# Validates:
#   1. Explicit workersAddr/peersAddr flow into the generated JSON
#      and ExecStart points at bin/control_plane
#   2. Cluster-ref workers resolve deployment.addr + labels
#   3. Unknown worker name in `workers` fails assertions (fail-fast)
#
# Run:
#   nix eval --raw -f ./nix/tests/control-plane-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows extractExecStartConfig;

  pkgsStub = mkPkgsStub { packages = ["control_plane"]; inherit system; };

  evalCp = cpCfg: clusterVms:
    lib.evalModules {
      modules = [
        ../modules/control-plane.nix
        ../modules/cluster.nix
        stubSystemd
        ({ ... }: {
          config.services.procurator.control-plane = cpCfg;
          config.cluster.vms = clusterVms;
        })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  cpConfig = evalResult:
    extractExecStartConfig
      evalResult.config.systemd.services."procurator-control-plane".serviceConfig.ExecStart
      "${toString pkgsStub.control_plane}/bin/control_plane ";

  mkClusterVm = addr: labels: {
    role = "worker";
    cpu = 1.0;
    memory = { amount = 1.0; unit = "GB"; };
    labels = labels;
    deployment = { addr = addr; };
  };

in
let
  # ── 1. Explicit workersAddr + peersAddr ────────────────────────────
  explicit = evalCp
    {
      enable = true;
      addr = "0.0.0.0:8080";
      hostname = "cp-1";
      workersAddr = [
        { id = "w1"; addr = "192.168.1.11:9080"; labels = [ "gpu" ]; }
        { id = "w2"; }
      ];
      peersAddr = [ "192.168.1.10:8080" ];
    }
    { };
  explicitJson = cpConfig explicit;

  explicitCheck =
    assert' "explicit passes assertions" (checkAssertions explicit)
    && assertEq "hostname" explicitJson.hostname "cp-1"
    && assertEq "addr" explicitJson.addr "0.0.0.0:8080"
    && assertEq "workers" explicitJson.workers [
      { id = "w1"; addr = "192.168.1.11:9080"; labels = [ "gpu" ]; }
      { id = "w2"; addr = null; labels = []; }
    ]
    && assertEq "peers_addr" explicitJson.role.Master.peers_addr [ "192.168.1.10:8080" ];

  # ── 2. Cluster-referenced workers resolve addr + labels ────────────
  clustered = evalCp
    {
      enable = true;
      addr = "0.0.0.0:8080";
      hostname = "cp-1";
      workers = [ "worker-1" ];
    }
    {
      worker-1 = mkClusterVm "192.168.1.11:9080" [ "gpu" ];
    };
  clusteredJson = cpConfig clustered;

  clusterCheck =
    assert' "clustered passes assertions" (checkAssertions clustered)
    && assertEq "clustered workers" clusteredJson.workers [
      { id = "worker-1"; addr = "192.168.1.11:9080"; labels = [ "gpu" ]; }
    ];

  # ── 3. Unknown worker name fails fast ──────────────────────────────
  unknownWorker = assertThrows "unknown worker name"
    (checkAssertions (evalCp
      {
        enable = true;
        addr = "0.0.0.0:8080";
        workers = [ "nope" ];
      }
      { }));

in
explicitCheck && clusterCheck && unknownWorker
