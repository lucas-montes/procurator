use serde::de::DeserializeOwned;

use super::{Error, supervisor::CreateCommand};


/// This is the interface that should create process and talk to the hypervisor or whatever backend we are using to manage vms/vmms
/// The questions is, do all backends need a client and a process like cloud hypervisor?
pub trait Factory: Clone + 'static + std::fmt::Debug {
    //NOTE: we need the Clone + 'static either way
    type VmHandle: Handle + Send + 'static + Sync;
    type Config: DeserializeOwned + std::fmt::Debug;

    /// The capnp backend config type that parameterizes `VmSpec(BackendConfig)` on the wire.
    type BackendConfig: ::capnp::traits::Owned + 'static;

    /// Domain type extracted from the wire format. Each backend defines how to convert
    /// from `vm_spec::Reader<'a, Self::BackendConfig>` into this type.
    type CreateVmSpec<'a>: TryFrom<
            commands::common_capnp::vm_spec::Reader<'a, Self::BackendConfig>,
            Error = capnp::Error,
        >;

    fn create_id() -> String; //TODO: maybe this should be a tpye of the interface?

    fn create_vm(
        &self,
        source: Self::CreateVmSpec<'_>,
    ) -> impl Future<Output = Result<CreateCommand<Self>, Error>>
    where
        Self: Sized;
}

#[derive(Debug)]
pub enum HandleError {
    Start(String),
    Cleanup(String),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleError::Start(msg) => write!(f, "start failed: {}", msg),
            HandleError::Cleanup(msg) => write!(f, "cleanup failed: {}", msg),
        }
    }
}

impl std::error::Error for HandleError {}

/// This is the interface used to communicate with the the VM itself, either a process in case of cloud hypervisor or whatever else is needed
/// However we might want an id? is good enough to have it in the registry.s map only?
/// A handle that holds the process running the VM and the client to communicate with him.
/// maybe don't needed actually, or yes because the registry needs to hold it and do stuff with it. at least check that everything is running ok, start, stop and other stuff from either the registry or the server
pub trait Handle
where
    Self: Sized,
{

    fn ip(&self) -> &str;

    fn start(&self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn delete(self) -> impl Future<Output = Result<(), HandleError>> + Send;

    fn health(&self) -> impl Future<Output = Result<(), HandleError>> + Send;
}
