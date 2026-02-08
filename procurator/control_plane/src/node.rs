use std::net::SocketAddr;

use tokio::sync::mpsc::Receiver;

use crate::dto::{NodeEvent, NodeMessage};

///! Node that handles communications between the server and the logic handled by the control plane.

pub struct Node {
    /// Channel to receive messages from the server
    node_channel: Receiver<NodeMessage>,
    peers_addr: Vec<SocketAddr>,
}

impl Node {
    pub fn new(node_channel: Receiver<NodeMessage>, peers_addr: Vec<SocketAddr>) -> Self {
        Node {
            node_channel,
            peers_addr,
        }
    }

    /// Main loop that processes messages from the server and sends command to the workers and orchestrates tasks
    pub async fn run(mut self) {
        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply { file, name } => {
                    // let _ = self.worker_channel.send(ControlPlaneMessage::Create).await;
                }
            }
        }
    }
}
