@0x9663f4dd604afe36;

struct Empty {}

struct Result(Ok, Err) {
  union {
    ok @0 :Ok;
    err @1 :Err;
  }
}

struct VmSpec(BackendConfig) {
  spec @0 :BackendConfig;
}

# Running VM observed on a worker
struct RunningVm {
  id @0 :Text;
  contentHash @1 :Text;             # Hash of running image
  status @2 :Text;                  # "running", "stopping", "failed", "restarting"
  uptime @3 :UInt64;                # Seconds
  metrics @4 :VmMetrics;
}

struct VmMetrics {
  cpuUsage @0 :Float32;             # 0.0 - 1.0 (as fraction of available)
  memoryUsage @1 :UInt64;           # Bytes
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
  healthy @1 :Bool;                 # Last heartbeat within threshold?
  generation @2 :UInt64;            # Highest generation worker has seen
  runningVms @3 :UInt32;            # Count of running VMs
  availableResources @4 :Resources;
  metrics @5 :WorkerMetrics;
}

struct VmStatus {
  id @0 :Text;
  workerId @1 :Text;                # Where it should/is running
  desiredHash @2 :Text;             # Master's desired image hash
  observedHash @3 :Text;            # Worker's observed image hash
  status @4 :Text;                  # "pending", "running", "stopping", "failed", "drifted"
  drifted @5 :Bool;                 # desiredHash != observedHash?
  metrics @6 :VmMetrics;
  ip @7 :Text;
  externalEndpoint @8 :Text;        # External URL to reach the VM (e.g., "https://localhost:8443/vm/<id>/")
}

struct Generation {
  number @0 :UInt64;
  commit @1 :Text;
  intentHash @2 :Text;
  timestamp @3 :UInt64;             # Unix seconds
  isActive @4 :Bool;
}

struct Resources {
  cpu @0 :Float32;
  memoryBytes @1 :UInt64;
}

struct Assignment(BackendConfig) {
  generation @0 :UInt64;        # Current master generation
  desiredVms @1 :List(VmSpec(BackendConfig));  # Full specs for this worker's VMs
}

struct ClusterStatus {
  activeGeneration @0 :UInt64;
  activeCommit @1 :Text;
  convergencePercent @2 :UInt32;    # % of desired state realized
  workers @3 :List(WorkerStatus);
  vms @4 :List(VmStatus);
}
