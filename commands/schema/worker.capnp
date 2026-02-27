@0x9663f4dd604afa36;

using Common = import "common.capnp";

# Interface for the worker process that runs on each node, manages VMs and reports status back to the master
interface Worker {
  read @0 () -> (data :Common.WorkerStatus);
  listVms @1 () -> (vms :List(Common.VmStatus));
  createVm @2 (spec :Common.VmSpec) -> (id :Text);
  deleteVm @3 (id :Text) -> ();
}
