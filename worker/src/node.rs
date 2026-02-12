use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc::Receiver;

use crate::dto::{NodeEvent, NodeMessage};
use crate::vms::VmManager;

///! Node that handles communications between the server and the logic handled by the control plane.

pub struct Node {
    /// Channel to receive messages from the server
    node_channel: Receiver<NodeMessage>,
    master_addr: SocketAddr,
    vm_manager: Arc<VmManager>,
}

impl Node {
    pub fn new(node_channel: Receiver<NodeMessage>, master_addr: SocketAddr, vm_manager: Arc<VmManager>) -> Self {
        Node {
            node_channel,
            master_addr,
            vm_manager,
        }
    }

    /// Main loop that processes messages from the server and sends command to the workers and orchestrates tasks
    pub async fn run(mut self) {
        tracing::info!(master_addr=?self.master_addr, "Node started with master addr");

        // Start metrics polling
        let _metrics_thread = self.vm_manager.start_metrics_polling();

        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply => {
                    tracing::info!("Received Apply event");
                    // TODO: Implement reconciliation logic
                    // See INTEGRATION.md for full example
                }
            }
        }
    }
}
