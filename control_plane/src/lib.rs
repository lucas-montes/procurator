use tokio::{sync::mpsc::channel, task};
use worker::Server;


mod scheduler;
mod server;
mod dto;
mod node;


pub async fn main() {
    let (worker_channel_tx, worker_channel_rx) = channel(100);
    let (node_channel_tx, node_channel_rx) = channel(100);

    let worker = Server::new(worker_channel_rx);
    let node = node::Node::new(node_channel_rx, worker_channel_tx);
    let control_plane = server::Server::new(node_channel_tx);

    task::spawn(node.run()).await.unwrap();
    task::spawn(worker.run()).await.unwrap();
    task::LocalSet::new()
        .run_until(async move {
            let resutl = task::spawn_local(
                control_plane.serve("127.0.0.1:3000".parse().expect("wrong addr")),
            )
            .await
            .unwrap();
            if let Err(e) = resutl {
                eprintln!("Error starting control plane server: {}", e);
            }
        })
        .await;
}
