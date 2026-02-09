use tokio::sync::{
    mpsc::Sender,
    oneshot::{self, Receiver},
};

pub enum NodeEvent{Apply}

pub enum NodeError {}

pub type NodeResult = Result<(), NodeError>;

pub struct NodeMessage {
    event: NodeEvent,
    sender: oneshot::Sender<NodeResult>,
}

impl NodeMessage {
    pub fn new(event: NodeEvent) -> (NodeReceiver, Self) {
        let (sender, rx) = oneshot::channel();
        let msg = Self { event, sender };
        (NodeReceiver(rx), msg)
    }

    pub fn event(&self) -> &NodeEvent {
        &self.event
    }

    pub fn reply(self, result: NodeResult) {
        let _ = self.sender.send(result);
    }
}


pub struct NodeReceiver(Receiver<NodeResult>);


#[derive(Clone)]
pub struct NodeMessenger(Sender<NodeMessage>);

impl From<Sender<NodeMessage>> for NodeMessenger {
    fn from(value: Sender<NodeMessage>) -> Self {
        Self(value)
    }
}
