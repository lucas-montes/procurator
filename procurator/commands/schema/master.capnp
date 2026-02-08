@0x9663f4dd604afd36;

using Common = import "common.capnp";
using WorkerModule = import "worker.capnp";

interface Master {
  # CD platform publishes new commits and desired cluster state
  publishState @0 (
    commit :Text,
    generation :UInt64,
    intentHash :Text,
    vmSpecs :List(Common.VmSpec)
  ) -> (result :Common.Result(Common.Empty, Text));

  # Workers get assignments
  getAssignment @1 (
    workerId :Text,
    lastSeenGeneration :UInt64
  ) -> (result :Common.Result(Common.Assignment, Text));

  # Workers push observability data
  pushData @2 (
    workerId :Text,
    observedGeneration :UInt64,
    runningVms :List(Common.RunningVm),
    metrics :Common.WorkerMetrics
  ) -> (result :Common.Result(Common.Empty, Text));

  # CLI gets cluster status
  getClusterStatus @3 () -> (status :Common.ClusterStatus);

  # CLI gets worker capability
  getWorker @4 (workerId :Text) -> (worker :WorkerModule.Worker);

  # CLI gets VM capability
  getVm @5 (vmId :Text) -> (vm :WorkerModule.Worker.Vm);
}
