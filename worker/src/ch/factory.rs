use std::path::PathBuf;

use serde::Deserialize;

use crate::vmm::{Handle as VmHandle, Factory as VmFactory, CreateCommand, Error as VmError, VmSpecRef};

use super::{client::Client, process::Process};

pub struct Handle {
    vm_id: String,
    client: Client,
    process: Process,
    socket_path: PathBuf,
}

impl VmHandle for Handle {}

#[derive(Debug, Clone)]
pub struct Factory {
    socket_dir: PathBuf,
    ch_binary: PathBuf,
    bridge_name: String,
}


impl VmFactory for Factory {
    type VmHandle = Handle;
    type Config = Config;

    fn new(config: Self::Config) -> Self
        where
            Self: Sized {
        Self{
            socket_dir: config.socket_dir,
            ch_binary: config.binary_path,
            bridge_name: config.bridge_name,
        }
    }

    async fn create_vm<'a>(
        &self,
        spec: VmSpecRef<'a>,
    ) -> Result<CreateCommand<Self>, VmError> {
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
