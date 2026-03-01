# Fast test for evalCluster — pure eval, no NixOS build.
#
# Validates:
#   1. Basic cluster with valid profiles
#   2. Derived cpu/memoryMb from profiles
#   3. Summary stats (vmCount, totalCpu, totalMemoryMb)
#   4. Default deployment values
#   5. Validation: missing vmProfile throws
#   6. Validation: invalid vmProfile throws
#   7. Validation: missing deployment throws
#   8. Multi-VM cluster
#
# Run:
#   nix eval --json -f ./nix/tests/cluster-fast.nix
#
# Returns true on success, throws on failure.

let
  mkVmProfile = import ../lib/profile {};
  evalCluster = import ../lib/cluster {};

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

  # ── Test profiles ──────────────────────────────────────────────────
  sandbox = mkVmProfile {
    hostname = "sandbox";
    cpu = 4;
    memoryMb = 2048;
    allowedDomains = [ "api.openai.com" ];
  };

  ci = mkVmProfile {
    hostname = "ci-runner";
    cpu = 2;
    memoryMb = 1024;
    autoShutdown = true;
  };

  # ── 1. Basic single-VM cluster ─────────────────────────────────────
  singleVm = evalCluster {
    vms = {
      sandbox-1 = {
        vmProfile = sandbox;
        deployment = { addr = "10.0.0.1"; };
      };
    };
  };

  basicCluster =
    assertEq "single vm count" singleVm.vmCount 1
    && assertEq "single total cpu" singleVm.totalCpu 4
    && assertEq "single total memory" singleVm.totalMemoryMb 2048;

  # ── 2. Derived values from profile ─────────────────────────────────
  derivedValues =
    assertEq "vm cpu from profile" singleVm.vms.sandbox-1.cpu 4
    && assertEq "vm memoryMb from profile" singleVm.vms.sandbox-1.memoryMb 2048;

  # ── 3. Default deployment values ───────────────────────────────────
  defaultDeployment =
    assertEq "default backend" singleVm.vms.sandbox-1.deployment.backend "cloud-hypervisor"
    && assertEq "default sshUser" singleVm.vms.sandbox-1.deployment.sshUser "root"
    && assertEq "default sshPort" singleVm.vms.sandbox-1.deployment.sshPort 22
    && assertEq "default autoRollback" singleVm.vms.sandbox-1.deployment.autoRollback true
    && assertEq "default role" singleVm.vms.sandbox-1.role "worker"
    && assertEq "default labels" singleVm.vms.sandbox-1.labels []
    && assertEq "default replicas" singleVm.vms.sandbox-1.replicas 1;

  # ── 4. Multi-VM cluster ────────────────────────────────────────────
  multiVm = evalCluster {
    vms = {
      sandbox-1 = {
        vmProfile = sandbox;
        deployment = { addr = "10.0.0.1"; };
      };
      sandbox-2 = {
        vmProfile = sandbox;
        deployment = { addr = "10.0.0.2"; };
      };
      ci-runner = {
        vmProfile = ci;
        deployment = { addr = "10.0.0.3"; };
        role = "worker";
        labels = [ "ci" "ephemeral" ];
      };
    };
  };

  multiCluster =
    assertEq "multi vm count" multiVm.vmCount 3
    && assertEq "multi total cpu" multiVm.totalCpu 10  # 4 + 4 + 2
    && assertEq "multi total memory" multiVm.totalMemoryMb 5120  # 2048 + 2048 + 1024
    && assertEq "ci labels" multiVm.vms.ci-runner.labels [ "ci" "ephemeral" ];

  # ── 5. Custom deployment values ────────────────────────────────────
  customDeployment = evalCluster {
    vms = {
      test = {
        vmProfile = sandbox;
        deployment = {
          addr = "192.168.1.10";
          backend = "cloud-hypervisor";
          sshUser = "admin";
          sshPort = 2222;
          autoRollback = false;
        };
        role = "control-plane";
      };
    };
  };

  customDeploy =
    assertEq "custom addr" customDeployment.vms.test.deployment.addr "192.168.1.10"
    && assertEq "custom sshUser" customDeployment.vms.test.deployment.sshUser "admin"
    && assertEq "custom sshPort" customDeployment.vms.test.deployment.sshPort 2222
    && assertEq "custom autoRollback" customDeployment.vms.test.deployment.autoRollback false
    && assertEq "custom role" customDeployment.vms.test.role "control-plane";

  # ── 6. Validation: missing vmProfile ───────────────────────────────
  missingProfile = assertThrows "missing vmProfile"
    (evalCluster {
      vms.bad = {
        deployment = { addr = "10.0.0.1"; };
      };
    }).vms.bad.cpu;

  # ── 7. Validation: invalid vmProfile (not from mkVmProfile) ───────
  invalidProfile = assertThrows "invalid vmProfile"
    (evalCluster {
      vms.bad = {
        vmProfile = { hostname = "fake"; };  # missing _type
        deployment = { addr = "10.0.0.1"; };
      };
    }).vms.bad.cpu;

  # ── 8. Validation: missing deployment ──────────────────────────────
  missingDeployment = assertThrows "missing deployment"
    (evalCluster {
      vms.bad = {
        vmProfile = sandbox;
      };
    }).vms.bad.deployment.addr;

  # ── 9. Validation: missing deployment.addr ─────────────────────────
  missingAddr = assertThrows "missing deployment.addr"
    (evalCluster {
      vms.bad = {
        vmProfile = sandbox;
        deployment = {};
      };
    }).vms.bad.deployment.addr;

  # ── 10. Empty cluster ──────────────────────────────────────────────
  emptyCluster = evalCluster { vms = {}; };

  emptyCounts =
    assertEq "empty vm count" emptyCluster.vmCount 0
    && assertEq "empty total cpu" emptyCluster.totalCpu 0
    && assertEq "empty total memory" emptyCluster.totalMemoryMb 0;

in
  basicCluster
  && derivedValues
  && defaultDeployment
  && multiCluster
  && customDeploy
  && missingProfile
  && invalidProfile
  && missingDeployment
  && missingAddr
  && emptyCounts
