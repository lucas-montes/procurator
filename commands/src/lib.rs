mod commands_capnp {
    include!(concat!(env!("OUT_DIR"), "/commands_capnp.rs"));
}

pub use commands_capnp::*;

pub enum ControlPlaneEvent {
    Create,
}

pub enum WorkerEvent {
    Create,
}

pub struct ControlPlaneMessage {
    event: ControlPlaneEvent,
    sender: tokio::sync::oneshot::Sender<WorkerEvent>,
}
