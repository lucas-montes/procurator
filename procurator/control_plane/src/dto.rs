use std::ops::{Deref, DerefMut};

use tokio::sync::{
    mpsc::Sender,
    oneshot::{self, Receiver},
};

pub enum NodeEvent {
    Apply {
        file: String,
        name: String,
    },
}

pub type NodeResult = Result<(), String>;

pub struct NodeMessage {
    event: NodeEvent,
    sender: tokio::sync::oneshot::Sender<NodeResult>,
}

impl NodeMessage {
    fn new(event: NodeEvent) -> (NodeReceiver, Self) {
        let (sender, rx) = oneshot::channel();
        let msg = Self { event, sender };
        (NodeReceiver(rx), msg)
    }

    pub fn event(&self) -> &NodeEvent {
        &self.event
    }
}

pub struct NodeReceiver(Receiver<NodeResult>);

impl Deref for NodeReceiver {
    type Target = Receiver<NodeResult>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NodeReceiver {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct NodeMessenger(Sender<NodeMessage>);

impl NodeMessenger {
    pub async fn apply(self, file: String, name: String) -> NodeReceiver {
        let (rx, msg) = NodeMessage::new(NodeEvent::Apply{file, name});
        self.0.send(msg).await.expect("message failed"); //TODO: handle better
        rx
    }
}

impl From<Sender<NodeMessage>> for NodeMessenger {
    fn from(value: Sender<NodeMessage>) -> Self {
        Self(value)
    }
}
