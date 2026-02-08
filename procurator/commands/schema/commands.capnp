@0x9663f4dd604afa36;

# ============================================================================
# Publisher Interface (Nix evaluation server -> master)
# ============================================================================

interface DesiredStatePublisher {
  publishDesiredState @0 (
    commit :Text,
    generation :UInt64,
    intentHash :Text,
    vmSpecs :List(VmSpec)
  ) -> (result :Result(Empty, Text));
}

# ============================================================================
# Master Control (user/CLI -> master)
# ============================================================================

interface MasterControl {
  getClusterStatus @0 () -> (status :ClusterStatus);
  getWorker @1 (workerId :Text) -> (worker :Worker);
  getVm @2 (vmId :Text) -> (vm :Vm);
}

# ============================================================================
# Worker Control (worker -> master)
# ============================================================================

interface WorkerControl {
  getAssignment @0 (
    workerId :Text,
    lastSeenGeneration :UInt64
  ) -> (result :Result(Assignment, Text));

  pushObservedState @1 (
    workerId :Text,
    observedGeneration :UInt64,
    runningVms :List(RunningVm),
    metrics :WorkerMetrics
  ) -> (result :Result(Empty, Text));
}

# ============================================================================
# Resource Interfaces (capabilities returned by master)
# ============================================================================

interface Worker {
  read @0 () -> (data :WorkerStatus);
  listVms @1 () -> (vms :List(VmStatus));
  getVm @2 (vmId :Text) -> (vm :Vm);
}

interface Vm {
  read @0 () -> (data :VmStatus);
  getLogs @1 (follow :Bool, tailLines :UInt32) -> (logs :VmLogs);
  exec @2 (command :Text, args :List(Text)) -> (output :ExecOutput);
  getConnectionInfo @3 () -> (info :ConnectionInfo);
}

# ============================================================================
# Generic Result Type
# ============================================================================
struct Empty {}

struct Result(Ok, Err) {
  union {
    ok @0 :Ok;
    err @1 :Err;
  }
}

# ============================================================================
# Data Structures
# ============================================================================

struct VmSpec {
  id @0 :Text;
  name @1 :Text;
  storePath @2 :Text;
  contentHash @3 :Text;
  cpu @4 :Float32;
  memoryBytes @5 :UInt64;
  labels @6 :List(Label);
  replicas @7 :UInt32;
  networkAllowedDomains @8 :List(Text);
}

struct Label {
  key @0 :Text;
  value @1 :Text;
}

struct Assignment {
  generation @0 :UInt64;
  desiredVms @1 :List(VmSpec);
}

struct RunningVm {
  id @0 :Text;
  contentHash @1 :Text;
  status @2 :Text;
  uptime @3 :UInt64;
  metrics @4 :VmMetrics;
}

struct VmMetrics {
  cpuUsage @0 :Float32;
  memoryUsage @1 :UInt64;
  networkRxBytes @2 :UInt64;
  networkTxBytes @3 :UInt64;
}

struct WorkerMetrics {
  availableCpu @0 :Float32;
  availableMemory @1 :UInt64;
  diskUsage @2 :UInt64;
  uptime @3 :UInt64;
}

struct WorkerStatus {
  id @0 :Text;
  healthy @1 :Bool;
  generation @2 :UInt64;
  runningVms @3 :UInt32;
  availableResources @4 :Resources;
  metrics @5 :WorkerMetrics;
}

struct VmStatus {
  id @0 :Text;
  workerId @1 :Text;
  desiredHash @2 :Text;
  observedHash @3 :Text;
  status @4 :Text;
  drifted @5 :Bool;
  metrics @6 :VmMetrics;
}

struct ClusterStatus {
  activeGeneration @0 :UInt64;
  activeCommit @1 :Text;
  convergencePercent @2 :UInt32;
  workers @3 :List(WorkerStatus);
  vms @4 :List(VmStatus);
}

struct Resources {
  cpu @0 :Float32;
  memoryBytes @1 :UInt64;
}

struct VmLogs {
  logs @0 :Text;
  truncated @1 :Bool;
}

struct ExecOutput {
  stdout @0 :Text;
  stderr @1 :Text;
  exitCode @2 :Int32;
}

struct ConnectionInfo {
  vmId @0 :Text;
  workerHost @1 :Text;
  sshPort @2 :UInt16;
  consolePort @3 :UInt16;
  username @4 :Text;
}
