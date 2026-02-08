use std::net::SocketAddr;

use tokio::sync::mpsc::Receiver;

use crate::dto::{WorkerEvent, WorkerMessage};
use crate::runtime::Controller;

/// Node that handles communications between the server and the logic handled by the worker.
/// Receives messages from the RPC server and delegates to the Controller.
pub struct Node {
    /// Channel to receive messages from the server
    worker_channel: Receiver<WorkerMessage>,
    /// Master node address (for future communication)
    #[allow(dead_code)]
    master_addr: SocketAddr,
    /// Local controller for VM management
    #[allow(dead_code)]
    controller: Controller,
}

impl Node {
    pub fn new(worker_channel: Receiver<WorkerMessage>, master_addr: SocketAddr) -> Self {
        Node {
            worker_channel,
            master_addr,
            controller: Controller::default(),
        }
    }

    /// Main loop that processes messages from the server and delegates to the controller
    pub async fn run(mut self) {
        while let Some(message) = self.worker_channel.recv().await {
            match message.event() {
                WorkerEvent::GetAssignment {
                    worker_id,
                    last_seen_generation,
                } => {
                    // TODO: Contact master to get assignment
                    tracing::info!(
                        worker_id = %worker_id,
                        generation = last_seen_generation,
                        "Requesting assignment from master"
                    );
                    message.reply(crate::dto::WorkerResult::GetAssignment(Err(
                        "not implemented".to_string(),
                    )));
                }
                WorkerEvent::PushObservedState { running_vms, metrics: _ } => {
                    // TODO: Gather actual state from controller and push to master
                    tracing::info!(
                        vm_count = running_vms.len(),
                        "Pushing observed state to master"
                    );
                    message.reply(crate::dto::WorkerResult::PushObservedState(Err(
                        "not implemented".to_string(),
                    )));
                }
                WorkerEvent::GetVmLogs { vm_id, follow, tail_lines } => {
                    // TODO: Use controller to fetch VM logs
                    tracing::info!(
                        vm_id = %vm_id,
                        follow = follow,
                        tail_lines = tail_lines,
                        "Fetching VM logs"
                    );
                    message.reply(crate::dto::WorkerResult::GetVmLogs(Err(
                        "not implemented".to_string(),
                    )));
                }
                WorkerEvent::ExecInVm { vm_id, command, args } => {
                    // TODO: Use controller to execute command in VM
                    tracing::info!(
                        vm_id = %vm_id,
                        command = %command,
                        arg_count = args.len(),
                        "Executing command in VM"
                    );
                    message.reply(crate::dto::WorkerResult::ExecInVm(Err(
                        "not implemented".to_string(),
                    )));
                }
                WorkerEvent::GetVmConnectionInfo { vm_id } => {
                    // TODO: Use controller to get VM connection info
                    tracing::info!(vm_id = %vm_id, "Getting VM connection info");
                    message.reply(crate::dto::WorkerResult::GetVmConnectionInfo(Err(
                        "not implemented".to_string(),
                    )));
                }
            }
        }

        tracing::warn!("Worker node channel closed, stopping");
    }
}
