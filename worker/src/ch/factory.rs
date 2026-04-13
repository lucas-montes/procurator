use std::path::PathBuf;

use serde::Deserialize;

use crate::vmm::{CreateCommand, Error as VmError, Factory as VmFactory, Handle as VmHandle};

use super::{client::Client, dtos::CreateVmSpecRef, process::Process};

pub struct Handle {
    vm_id: String,
    client: Client,
    process: Process,
    socket_path: PathBuf,
}

impl VmHandle for Handle {}

/// The structure responsible to spin up vm managed with cloud hypervisor. We need the socket and the binary to be able to call and communicate
/// with the child processes. We also need to have a bridge name to be able to attach the TAP interfaces to it. I don't really mind cloning it,
/// even if maybe using an Arc would be better? Not, sure, some path we need to modify them to add the uuid to identify each vmm/vm, but not the
/// binary path nor the bridge_name. Also instead of cloning them I should have a function that returns a hadnle, which whil hold the logic
/// to make the vm do stuff?
#[derive(Debug, Clone)]
pub struct Factory {
    socket_dir: PathBuf,
    ch_binary: PathBuf,
    bridge_name: String,
}

impl From<Config> for Factory {
    fn from(config: Config) -> Self {
        Self {
            socket_dir: config.socket_dir,
            ch_binary: config.binary_path,
            bridge_name: config.bridge_name,
        }
    }
}

impl VmFactory for Factory {
    type VmHandle = Handle;
    type Config = Config;
    type BackendConfig = commands::ch_capnp::vm_config::Owned;
    type CreateVmSpec<'a> = CreateVmSpecRef<'a>;

    async fn create_vm(
        &self,
        source: Self::CreateVmSpec<'_>,
    ) -> Result<CreateCommand<Self>, VmError> {
        // let _spec = CreateVmSpecRef::try_from(source)
        //     .map_err(|e| VmError::Internal(format!("Invalid VM spec: {e}")))?;

        // 1) generate vm id
        // 2) prepare vm dir + writable disk + tap
        // 3) spawn cloud-hypervisor process
        // 4) wait for socket
        // 5) build Client and call client.create(config) + client.boot()
        // 6) build ChHandle and return CreateCommand::new(handle, vm_id)

        // ...existing code...
        unimplemented!()
    }

    async fn delete_vm(&self, id: &str) -> Result<(), VmError> {
        // cleanup durable artifacts for VM id if needed
        // ...existing code...
        let _ = id;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    binary_path: PathBuf,
    socket_dir: PathBuf,
    socket_timeout_secs: u64,
    bridge_name: String,
}
