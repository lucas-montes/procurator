use std::net::SocketAddr;

use tokio::sync::mpsc::Receiver;

use crate::dto::{NodeEvent, NodeMessage};
use crate::vmm::Vmm;

///! Node that handles communications between the server and the logic handled by the control plane.

pub struct Node<V: Vmm> {
    /// Channel to receive messages from the server
    node_channel: Receiver<NodeMessage>,
    master_addr: SocketAddr,
    /// VMM backend for managing virtual machines
    vmm: V,
}

impl<V: Vmm> Node<V> {
    pub fn new(
        node_channel: Receiver<NodeMessage>,
        master_addr: SocketAddr,
        vmm: V,
    ) -> Self {
        Node {
            node_channel,
            master_addr,
            vmm,
        }
    }

    /// Main loop that processes messages from the server and sends command to the workers and orchestrates tasks
    pub async fn run(mut self) {
        tracing::info!(master_addr=?self.master_addr, "Node started with master addr");
        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply => todo!(),
            }
        }
    }
}
