
/// This is the interface that should create process and talk to the hypervisor or whatever backend we are using to manage vms/vmms
/// The questions is, do all backends need a client and a process like cloud hypervisor?
pub trait Factory {
    type VmHandle: Handle;
    type Error;
}

/// This is the interface used to communicate with the the VM itself, either a process in case of cloud hypervisor or whatever else is needed
/// However we might want an id? is good enough to have it in the registry.s map only?
/// A handle that holds the process running the VM and the client to communicate with him.
/// maybe don't needed actually, or yes because the registry needs to hold it and do stuff with it. at least check that everything is running ok, start, stop and other stuff from either the registry or the server
pub trait Handle {}
