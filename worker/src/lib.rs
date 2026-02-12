use std::net::SocketAddr;
use std::sync::Arc;

use tokio::{sync::mpsc::channel, task};

use crate::{node::Node, server::Server};
use crate::vms::{VmManager, VmManagerConfig};

pub mod vms;
mod dto;
mod node;
mod server;

pub async fn main(_hostname: String, addr: SocketAddr, master_addr: SocketAddr) {
    // Initialize VM manager
    let vm_config = VmManagerConfig::default();
    let vm_manager = match VmManager::new(vm_config) {
        Ok(mgr) => Arc::new(mgr),
        Err(e) => {
            tracing::error!(?e, "Failed to create VM manager");
            return;
        }
    };

    tracing::info!("VM manager initialized");

    let (tx, rx) = channel(100);

    let node = Node::new(rx, master_addr, vm_manager);
    let server = Server::new(tx);

    tracing::info!(?addr, "Starting worker server",);

    let node_task = task::spawn(node.run());

    task::LocalSet::new()
        .run_until(async move {
            tracing::info!("Internal localset server");
            let resutl = task::spawn_local(server.serve(addr)).await;
            match resutl {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Error starting worker server"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        })
        .await;

    if let Err(err) = node_task.await {
        tracing::error!(?err, "Node task panicked");
    }
}
