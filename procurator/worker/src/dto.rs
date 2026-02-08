use std::ops::{Deref, DerefMut};

use tokio::sync::{
    mpsc::Sender,
    oneshot::{self, Receiver},
};
use commands::{Assignment, RunningVmObserved, WorkerMetricsObserved, VmLogs, ExecOutput, ConnectionInfo};

/// Events that the Node can handle
pub enum WorkerEvent {
    /// Get current assignment from master
    GetAssignment {
        worker_id: String,
        last_seen_generation: u64,
    },
    /// Push observed state to master
    PushObservedState {
        running_vms: Vec<RunningVmObserved>,
        #[allow(dead_code)]
        metrics: WorkerMetricsObserved,
    },
    /// Get logs from a specific VM
    GetVmLogs {
        vm_id: String,
        follow: bool,
        tail_lines: u32,
    },
    /// Execute command in a VM
    ExecInVm {
        vm_id: String,
        command: String,
        args: Vec<String>,
    },
    /// Get VM connection info
    GetVmConnectionInfo {
        vm_id: String,
    },
}

#[allow(dead_code)]
pub enum WorkerResult {
    GetAssignment(Result<Assignment, String>),
    PushObservedState(Result<(), String>),
    GetVmLogs(Result<VmLogs, String>),
    ExecInVm(Result<ExecOutput, String>),
    GetVmConnectionInfo(Result<ConnectionInfo, String>),
}

pub struct WorkerMessage {
    event: WorkerEvent,
    sender: tokio::sync::oneshot::Sender<WorkerResult>,
}

impl WorkerMessage {
    #[allow(dead_code)]
    fn new(event: WorkerEvent) -> (WorkerReceiver, Self) {
        let (sender, rx) = oneshot::channel();
        let msg = Self { event, sender };
        (WorkerReceiver(rx), msg)
    }

    pub fn event(&self) -> &WorkerEvent {
        &self.event
    }

    pub fn reply(self, result: WorkerResult) {
        let _ = self.sender.send(result);
    }
}

pub struct WorkerReceiver(Receiver<WorkerResult>);

impl Deref for WorkerReceiver {
    type Target = Receiver<WorkerResult>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WorkerReceiver {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct WorkerMessenger(#[allow(dead_code)] Sender<WorkerMessage>);

impl WorkerMessenger {
    #[allow(dead_code)]
    pub async fn get_assignment(
        self,
        worker_id: String,
        last_seen_generation: u64,
    ) -> WorkerReceiver {
        let (rx, msg) = WorkerMessage::new(WorkerEvent::GetAssignment {
            worker_id,
            last_seen_generation,
        });
        self.0.send(msg).await.expect("message failed");
        rx
    }

    #[allow(dead_code)]
    pub async fn push_observed_state(
        self,
        running_vms: Vec<RunningVmObserved>,
        metrics: WorkerMetricsObserved,
    ) -> WorkerReceiver {
        let (rx, msg) = WorkerMessage::new(WorkerEvent::PushObservedState {
            running_vms,
            metrics,
        });
        self.0.send(msg).await.expect("message failed");
        rx
    }

    #[allow(dead_code)]
    pub async fn get_vm_logs(self, vm_id: String, follow: bool, tail_lines: u32) -> WorkerReceiver {
        let (rx, msg) =
            WorkerMessage::new(WorkerEvent::GetVmLogs { vm_id, follow, tail_lines });
        self.0.send(msg).await.expect("message failed");
        rx
    }

    #[allow(dead_code)]
    pub async fn exec_in_vm(
        self,
        vm_id: String,
        command: String,
        args: Vec<String>,
    ) -> WorkerReceiver {
        let (rx, msg) = WorkerMessage::new(WorkerEvent::ExecInVm { vm_id, command, args });
        self.0.send(msg).await.expect("message failed");
        rx
    }

    #[allow(dead_code)]
    pub async fn get_vm_connection_info(self, vm_id: String) -> WorkerReceiver {
        let (rx, msg) = WorkerMessage::new(WorkerEvent::GetVmConnectionInfo { vm_id });
        self.0.send(msg).await.expect("message failed");
        rx
    }
}

impl From<Sender<WorkerMessage>> for WorkerMessenger {
    fn from(value: Sender<WorkerMessage>) -> Self {
        Self(value)
    }
}
