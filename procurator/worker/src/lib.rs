use std::net::SocketAddr;

use tokio::{sync::mpsc::channel, task};

use crate::server::Server;
use crate::node::Node;

mod dto;
mod node;
mod runtime;
mod server;
mod builder;


pub async fn main(_hostname: String, addr: SocketAddr, master_addr: SocketAddr) {
    let (tx, rx) = channel(100);

    let node = Node::new(rx, master_addr);
    let server = Server::new(tx);

    task::spawn(node.run()).await.expect("Node task panicked");

    task::LocalSet::new()
        .run_until(async move {
            let result = task::spawn_local(server.serve(addr)).await;
            match result {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Error starting worker server"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        })
        .await;
}
