pub enum ControlPlaneEvent {
    Create,
}

pub enum WorkerEvent {
    Create,
}

pub struct NodeMessage {
    event: ControlPlaneEvent,
    sender: tokio::sync::oneshot::Sender<WorkerEvent>,
}
