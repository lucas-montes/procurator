@0x9663f4dd604afe36;

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

# Desired state for a single VM (output of Nix evaluation)
struct VmSpec {
  id @0 :Text;                      # Unique VM logical ID
  name @1 :Text;                    # Human-readable name
  storePath @2 :Text;               # /nix/store/... path to VM image
  contentHash @3 :Text;             # Hash of VM image for drift detection
  cpu @4 :Float32;                  # Fractional vCPUs (0.5, 1.0, etc.)
  memoryBytes @5 :UInt64;           # RAM in bytes
  labels @6 :List(Label);           # For scheduling constraints/affinity
  replicas @7 :UInt32;              # How many copies should run
  networkAllowedDomains @8 :List(Text);  # If empty, no network (isolated)
}

struct Label {
  key @0 :Text;
  value @1 :Text;
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

struct Assignment {
  generation @0 :UInt64;        # Current master generation
  desiredVms @1 :List(VmSpec);  # Full specs for this worker's VMs
}

struct ClusterStatus {
  activeGeneration @0 :UInt64;
  activeCommit @1 :Text;
  convergencePercent @2 :UInt32;    # % of desired state realized
  workers @3 :List(WorkerStatus);
  vms @4 :List(VmStatus);
}

struct VmLogs {
  logs @0 :Text;
  truncated @1 :Bool;     # If tailLines was used and more exist
}

struct ExecOutput {
  stdout @0 :Text;
  stderr @1 :Text;
  exitCode @2 :Int32;
}

struct ConnectionInfo {
  vmId @0 :Text;
  workerHost @1 :Text;    # Worker machine hostname/IP
  sshPort @2 :UInt16;     # SSH port forwarded from VM
  consolePort @3 :UInt16; # Serial console port (optional)
  username @4 :Text;      # Default SSH username
}
