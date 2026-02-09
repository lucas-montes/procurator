use std::net::SocketAddr;

use tokio::{sync::mpsc::channel, task};

use crate::{node::Node, server::Server};

mod dto;
mod node;
mod scheduler;
mod server;

pub async fn main(_hostname: String, addr: SocketAddr, peers_addr: Vec<SocketAddr>) {
    let (tx, rx) = channel(100);

    let node = Node::new(rx, peers_addr);
    let server = Server::new(tx);

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
