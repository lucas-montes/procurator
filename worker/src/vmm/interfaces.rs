use serde::de::DeserializeOwned;

use crate::supervisor::CreateCommand;
use crate::vmm::errors::Error;

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
    ) -> impl Future<Output = Result<CreateCommand<Self>, Error>> + Send
    where
        Self: Sized;
}
