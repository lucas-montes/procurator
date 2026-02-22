pub mod dto;
pub mod node;
pub mod server;
pub mod vmm;

use std::net::SocketAddr;

use node::Node;
use server::Server;
use tokio::sync::mpsc;

pub async fn main(_hostname: String, listen_addr: SocketAddr, master_addr: SocketAddr) {
    let (node_tx, node_rx) = mpsc::channel(100);

    // TODO: Make socket path configurable
    let socket_path = "/tmp/cloud-hypervisor.sock";
    let vmm = vmm::cloud_hypervisor::CloudHypervisor::new(socket_path);

    let server = Server::new(node_tx.clone(), vmm.clone());
    let node = Node::new(node_rx, master_addr, vmm);

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
