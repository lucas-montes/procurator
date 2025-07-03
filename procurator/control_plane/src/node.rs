use commands::ControlPlaneMessage;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::dto::{NodeMessage, NodeEvent};

///! Node that handles communications between the server and the logic handled by the control plane.

pub struct Node{
    /// Channel to receive messages from the server
    node_channel: Receiver<NodeMessage>,
    /// Channel to send messages to the worker
    worker_channel: Sender<ControlPlaneMessage>,
}

impl Node {
    pub fn new(node_channel: Receiver<NodeMessage>, worker_channel: Sender<ControlPlaneMessage>) -> Self {
        Node { node_channel, worker_channel }
    }

    /// Main loop that processes messages from the server and sends command to the workers and orchestrates tasks
    pub async fn run(mut self) {
        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply{file, name} => {
                        // let _ = self.worker_channel.send(ControlPlaneMessage::Create).await;
                }
            }
        }
    }
}
