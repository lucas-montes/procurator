//! Communicates with the API server, creates/deletes containers via the container runtime, and monitors pod health

use commands::ControlPlaneMessage;
use tokio::sync::mpsc::Receiver;

use crate::runtime::Controller;

pub struct Server {
    controller: Controller,
    control_plane_channel: Receiver<ControlPlaneMessage>,
}

impl Server {
    pub fn new(control_plane_channel: Receiver<ControlPlaneMessage>) -> Self {
        let controller = Controller::default();
        Server {
            controller,
            control_plane_channel,
        }
    }
    pub async fn run(mut self) {
        while let Some(message) = self.control_plane_channel.recv().await {
            todo!()
        }
    }
}
