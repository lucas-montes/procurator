use core::fmt;

use crate::vmm::supervisor::CreateCommand;

use super::VmSpecRef;

#[derive(Debug)]
pub enum Error {
    CreationFailed(String),
    DeletionFailed(String),
    OperationFailed(String),
    Communication(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CreationFailed(msg) => write!(f, "VM creation failed: {}", msg),
            Error::DeletionFailed(msg) => write!(f, "VM deletion failed: {}", msg),
            Error::OperationFailed(msg) => write!(f, "VM operation failed: {}", msg),
            Error::Communication(msg) => write!(f, "Communication error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for capnp::Error {
    fn from(e: Error) -> Self {
        capnp::Error::failed(e.to_string())
    }
}

/// This is the interface that should create process and talk to the hypervisor or whatever backend we are using to manage vms/vmms
/// The questions is, do all backends need a client and a process like cloud hypervisor?
pub trait Factory {
    type VmHandle: Handle + Send + 'static + Sync;

    fn create_vm(
        &self,
        spec: VmSpecRef,
    ) -> impl Future<Output = Result<CreateCommand<Self>, Error>>
    where
        Self: Sized;

    fn delete_vm(&self, id: &str) -> impl Future<Output = Result<(), Error>>;
}

/// This is the interface used to communicate with the the VM itself, either a process in case of cloud hypervisor or whatever else is needed
/// However we might want an id? is good enough to have it in the registry.s map only?
/// A handle that holds the process running the VM and the client to communicate with him.
/// maybe don't needed actually, or yes because the registry needs to hold it and do stuff with it. at least check that everything is running ok, start, stop and other stuff from either the registry or the server
pub trait Handle {}
