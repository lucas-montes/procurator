pub mod dto;
pub mod node;
pub mod server;
pub mod vm_manager;
pub mod vmm;

#[cfg(test)]
mod vm_manager_tests;

use std::net::SocketAddr;

use node::Node;
use server::Server;
use tokio::sync::mpsc;
use vm_manager::VmManagerConfig;
use vmm::cloud_hypervisor::{CloudHypervisorBackend, CloudHypervisorConfig};

use crate::dto::CommandSender;

pub async fn main(_hostname: String, listen_addr: SocketAddr, master_addr: SocketAddr) {
    let (cmd_tx, cmd_rx) = mpsc::channel(100);

    // Server only holds the sending end — no VMM, no state
    let server = Server::new(CommandSender::new(cmd_tx));

    // Backend handles process spawning, socket management, config building
    let backend = CloudHypervisorBackend::new(CloudHypervisorConfig::default());

    // Node owns the receiving end + VmManager with all VM state
    let manager_config = VmManagerConfig::default();
    let node = Node::new(cmd_rx, master_addr, backend, manager_config);

    tokio::select! {
        result = server.serve(listen_addr) => {
            if let Err(e) = result {
                tracing::error!(error = ?e, "Server failed");
            }
        }
        _ = node.run() => {
            tracing::info!("Node stopped");
        }
    }
}
