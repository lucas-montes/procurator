@0x9663f4dd604afa36;

using Common = import "common.capnp";

# Interface for the worker process that runs on each node, manages VMs and reports status back to the master
interface Worker {
  read @0 () -> (data :Common.WorkerStatus);
  listVms @1 () -> (vms :List(Common.VmStatus));
  getVm @2 (vmId :Text) -> (vm :Vm);

  # Interface for managing a specific VM. The master can ask the worker to exec commands in the VM or stream logs for debugging purposes
  interface Vm {
    read @0 () -> (data :Common.VmStatus);
    getLogs @1 (follow :Bool, tailLines :UInt32) -> (logs :Common.VmLogs);
    exec @2 (command :Text, args :List(Text)) -> (output :Common.ExecOutput);
    getConnectionInfo @3 () -> (info :Common.ConnectionInfo);
  }
}
