@0x9663f4dd604afa36;

using Common = import "common.capnp";

# Interface for the worker process that runs on each node, manages VMs and reports status back to the master
interface Worker(BackendConfig) {
  read @0 () -> (data :Common.WorkerStatus);
  listVms @1 () -> (vms :List(Common.VmStatus));
  createVm @2 (spec :Common.VmSpec(BackendConfig)) -> (id :Text);
  deleteVm @3 (id :Text) -> ();

  # Pause a running VM. Returns once the VM is fully paused.
  pauseVm @4 (id :Text) -> ();

  # Resume a previously paused VM. Fire-and-forget: the worker returns
  # immediately and runs the resume in the background.
  resumeVm @5 (id :Text) -> ();

  # Snapshot a VM (memory + device state) to `destination` on the worker.
  # The worker pauses the VM, snapshots, then resumes. Returns the absolute
  # path of the snapshot directory once it is ready.
  snapshotVm @6 (id :Text, destination :Text) -> (path :Text);

  # Copy a VM's writable disk to `destination` on the worker. The worker
  # pauses the VM for consistency, copies, then resumes. Returns the
  # absolute path of the disk image once it is ready.
  backupDisk @7 (id :Text, destination :Text) -> (path :Text);
}
