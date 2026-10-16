# Fast test for the worker NixOS service module — pure eval, no NixOS build.
#
# Validates:
#   1. workerId/masterAddr/intervals flow into the generated worker JSON
#   2. cluster-ref master resolves deployment.addr into master_addr
#   3. Missing workerId fails assertions (fail-fast at eval)
#   4. Missing master/masterAddr fails assertions
#
# Run:
#   nix eval --json -f ./nix/tests/worker-service-fast.nix
#
# Returns true on success, throws on failure. Needs a nixpkgs on NIX_PATH
# (only its pure `lib` is imported; no packages are evaluated or built).

let
  lib = import <nixpkgs/lib>;

  # Minimal pkgs stub: only what service.nix touches at eval time. writeText
  # returns the text itself so tests can parse the generated JSON; package
  # paths stay inert strings (nothing is built). `procurator` is a fake
  # inert derivation purely to satisfy `types.package` (never built: only its
  # out path is interpolated).
  pkgsStub = {
    writeText = _name: text: text;
    package = "pkg";
    runtimeShell = "/bin/sh";
    coreutils = "/bin";
    cloud-hypervisor = "/bin";
    procurator = derivation {
      name = "procurator-stub";
      builder = "/bin/sh";
      args = [ "-c" ":" ];
      system = builtins.currentSystem;
    };
    runtimeShellPackage = "/bin";
  };

  # The service module sets NixOS-core options (systemd/users/assertions)
  # which are out of scope here; declare them as unchecked so the module
  # evaluates standalone.
  stubSystemd = { lib, ... }: {
    options.systemd.services =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.users.users =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.users.groups =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.assertions = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          assertion = lib.mkOption { type = lib.types.bool; };
          message = lib.mkOption { type = lib.types.str; };
        };
      });
      default = [];
    };
  };

  # Force assertion checking (evalModules alone does not run them).
  checkAssertions = evalResult:
    let
      failed = builtins.filter (a: !a.assertion) evalResult.config.assertions;
    in
    if failed == []
    then true
    else builtins.throw ("ASSERTIONS FAILED: " + builtins.toJSON (map (a: a.message) failed));

  evalWorker = workerCfg: clusterVms:
    lib.evalModules {
      modules = [
        ../modules/worker/service.nix
        ../modules/cluster.nix
        stubSystemd
        ({ ... }: {
          config.services.procurator.worker = workerCfg;
          config.cluster.vms = clusterVms;
        })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  # The ExecStart string embeds the whole worker JSON after the binary path.
  # The stub derivation path leaks string context into it; discard the
  # context (test-only: the JSON content itself carries no store paths).
  workerJson = evalResult:
    let
      execStart = evalResult.config.systemd.services.procurator-worker.serviceConfig.ExecStart;
      prefix = "${pkgsStub.procurator}/bin/procurator ";
      json = builtins.substring
        (builtins.stringLength prefix)
        (builtins.stringLength execStart)
        execStart;
    in
    builtins.fromJSON (builtins.unsafeDiscardStringContext json);

  # ── Assertion helpers ──────────────────────────────────────────────
  assert' = msg: cond:
    if cond then true
    else builtins.throw "ASSERTION FAILED: ${msg}";

  assertEq = name: actual: expected:
    assert' "${name}: expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}"
      (actual == expected);

  assertThrows = name: expr:
    assert' "${name}: expected evaluation to throw, but it succeeded"
      (!(builtins.tryEval expr).success);

  # ── 1. Explicit workerId/masterAddr/intervals reach the JSON ─────────
  explicit = evalWorker
    {
      enable = true;
      listenAddr = "0.0.0.0:8080";
      workerId = "w1";
      masterAddr = "192.168.1.10:5000";
      pollIntervalMillis = 2500;
      heartbeatIntervalMillis = 7000;
    }
    { };
  explicitJson = workerJson explicit;

  explicitFields =
    assert' "valid config passes assertions" (checkAssertions explicit)
    && assertEq "worker_id" explicitJson.worker_id "w1"
    && assertEq "master_addr" explicitJson.master_addr "192.168.1.10:5000"
    && assertEq "poll_interval" explicitJson.poll_interval_millis 2500
    && assertEq "heartbeat_interval" explicitJson.heartbeat_interval_millis 7000
    && assertEq "rpc_listen_addr" explicitJson.rpc_listen_addr "0.0.0.0:8080";

  # ── 2. Cluster-ref master resolves deployment.addr ──────────────────
  clustered = evalWorker
    {
      enable = true;
      workerId = "worker-1";
      master = "control-plane-1";
    }
    {
      control-plane-1 = {
        role = "control-plane";
        cpu = 1.0;
        memory = { amount = 1.0; unit = "GB"; };
        deployment = { addr = "192.168.1.10:5000"; };
      };
      worker-1 = {
        role = "worker";
        cpu = 1.0;
        memory = { amount = 1.0; unit = "GB"; };
        deployment = { addr = "192.168.1.11:8080"; };
      };
    };
  clusteredJson = workerJson clustered;

  clusterRef =
    assert' "cluster config passes assertions" (checkAssertions clustered)
    && assertEq "cluster master_addr" clusteredJson.master_addr "192.168.1.10:5000"
    && assertEq "cluster worker_id" clusteredJson.worker_id "worker-1";

  # ── 3. Missing workerId fails fast ──────────────────────────────────
  missingWorkerId = assertThrows "missing workerId"
    (checkAssertions (evalWorker
      {
        enable = true;
        masterAddr = "192.168.1.10:5000";
      }
      { }));

  # ── 4. Missing master/masterAddr fails fast ─────────────────────────
  missingMaster = assertThrows "missing master"
    (checkAssertions (evalWorker
      {
        enable = true;
        workerId = "w1";
      }
      { }));

in
explicitFields && clusterRef && missingWorkerId && missingMaster
