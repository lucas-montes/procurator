//! Node — single owner of all VM state and logic.
//!
//! The Node holds the VmManager and a channel receiver. It processes
//! commands from the Server sequentially. No locks — the Node is the
//! only task that touches VmManager.
//!
//! Generic over [`VmmBackend`] so the entire stack can be tested
//! without real hypervisors.

use std::net::SocketAddr;

use tokio::sync::mpsc::Receiver;
use tracing::info;

use crate::dto::Message;
use crate::vm_manager::{VmManager, VmManagerConfig};
use crate::vmm::VmmBackend;

pub struct Node<B: VmmBackend> {
    /// Channel to receive commands from the Server
    commands: Receiver<Message>,
    /// Control plane address (for future reconciliation)
    master_addr: SocketAddr,
    /// Owns all VM state — generic over the VMM backend
    manager: VmManager<B>,
}

impl<B: VmmBackend> Node<B> {
    pub fn new(
        commands: Receiver<Message>,
        master_addr: SocketAddr,
        backend: B,
        manager_config: VmManagerConfig,
    ) -> Self {
        Node {
            commands,
            master_addr,
            manager: VmManager::new(backend, manager_config),
        }
    }

    /// Main loop — process commands until the channel is closed.
    pub async fn run(mut self) {
        info!(master_addr = ?self.master_addr, "Node started");

        while let Some(cmd) = self.commands.recv().await {
            self.manager.handle(cmd).await;
        }

        info!("Node command channel closed, shutting down");
    }
}
