use std::net::SocketAddr;

use tokio::{sync::mpsc::channel, task};

use crate::{node::Node, registry::DeclaredWorker, server::Server};

pub mod config;
pub mod directory;
mod dto;
mod embedded;
pub mod logging;
mod msglog;
mod node;
pub mod registry;
mod scheduler;
mod server;
mod store;

pub use embedded::{DesiredVm, PublishError, Publisher, VmImage, start};

pub async fn main<T: capnp::traits::Owned + 'static>(
    _hostname: String,
    addr: SocketAddr,
    peers_addr: Vec<SocketAddr>,
    declared: Vec<DeclaredWorker>,
    state_log_interval: std::time::Duration,
) {
    let (tx, rx) = channel(100);

    let node = Node::new(rx, peers_addr, declared).with_state_log_interval(state_log_interval);
    let server: Server<T> = Server::new(tx);

    tracing::info!(?addr, "Starting control plane server",);

    let node_task = task::spawn(node.run());

    task::LocalSet::new()
        .run_until(async move {
            tracing::info!("Internal localset server");
            let resutl = task::spawn_local(server.serve(addr)).await;
            match resutl {
                Ok(Ok(())) => tracing::info!("Control plane server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Error starting control plane server"),
                Err(err) => tracing::error!(?err, "Control plane server task panicked"),
            }
        })
        .await;

    if let Err(err) = node_task.await {
        tracing::error!(?err, "Node task panicked");
    }
}
