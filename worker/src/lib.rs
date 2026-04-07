pub mod dto;
pub mod server;
pub mod vm_manager;
pub mod vmm;

#[cfg(test)]
mod vm_manager_tests;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use server::Server;
use tokio::task;
use tokio::{join, sync::mpsc};
use vm_manager::{VmManager, VmManagerConfig};
use vmm::cloud_hypervisor::{CloudHypervisorBackend, CloudHypervisorConfig};

use crate::dto::CommandSender;

#[derive(Debug, Deserialize)]
pub struct CloudHypervisorSection {
    binary_path: PathBuf,
    socket_dir: PathBuf,
    socket_timeout_secs: u64,
    bridge_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    listen_addr: SocketAddr,
    master_addr: SocketAddr,
    cloud_hypervisor: CloudHypervisorSection,
}

pub async fn main(config: Config) {
    let (cmd_tx, cmd_rx) = mpsc::channel(100);

    // Server only holds the sending end — no VMM, no state
    let server = Server::new(CommandSender::new(cmd_tx));

    // Backend handles process spawning, socket management, config building.
    // All runtime settings come from the parsed config file.
    let ch_config = CloudHypervisorConfig {
        socket_dir: config.cloud_hypervisor.socket_dir,
        ch_binary: config.cloud_hypervisor.binary_path,
        socket_timeout: Duration::from_secs(config.cloud_hypervisor.socket_timeout_secs),
        bridge_name: config.cloud_hypervisor.bridge_name,
    };

    tracing::info!(
        ch_binary = %ch_config.ch_binary.display(),
        socket_dir = %ch_config.socket_dir.display(),
        socket_timeout_secs = ch_config.socket_timeout.as_secs(),
        bridge_name = ?ch_config.bridge_name,
        "Using cloud-hypervisor binary"
    );

    let backend = CloudHypervisorBackend::new(ch_config);

    // VmManager owns all VM state and handles commands sequentially.
    let manager_config = VmManagerConfig::default();
    let mut manager = VmManager::new(backend, manager_config);
    tracing::info!(master_addr = %config.master_addr, "Worker manager started");

    let manager_task = task::spawn(async move {
        let mut cmd_rx = cmd_rx;
        while let Some(msg) = cmd_rx.recv().await {
            manager.handle(msg).await;
        }
        tracing::info!("Worker manager command channel closed, shutting down");
    });

    // capnp-rpc requires spawn_local, which needs a LocalSet context
    let local_set = task::LocalSet::new();
    let server_task = local_set.run_until(task::spawn_local(server.serve(config.listen_addr)));

    match join!(manager_task, server_task) {
        (manager_result, server_result) => {
            if let Err(err) = manager_result {
                tracing::error!(?err, "Worker manager task panicked");
            }
            match server_result {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Worker server failed"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        }
    }
}
