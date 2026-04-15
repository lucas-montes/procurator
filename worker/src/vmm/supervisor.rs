use std::num::NonZeroU64;
use std::time::Duration;
use tokio::{select, sync::mpsc::Receiver, time::interval};

use super::{
    interfaces::Factory,
    registry::{Registry, Writer},
};

pub struct CreateCommand<F: Factory> {
    handle: F::VmHandle,
    id: String,
}

impl<F: Factory> CreateCommand<F> {
    pub fn new(handle: F::VmHandle, id: String) -> Self {
        Self { handle, id }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

pub enum Command<F: Factory> {
    Create(CreateCommand<F>),
    Delete(String),
}

/// Background structure that will save processes running, check their status, and make operations to the state if needed. It will be the primary writter for blocking operations
pub struct Supervisor<F: Factory> {
    state: Registry<F, Writer>,
    rx: Receiver<Command<F>>,
}

impl<F: Factory> Supervisor<F> {
    pub fn new(state: Registry<F, Writer>, rx: Receiver<Command<F>>) -> Self {
        Self { state, rx }
    }

    pub async fn run(mut self, health_tick_millis: NonZeroU64) {
        let period = Duration::from_millis(health_tick_millis.get());
        let mut ticker = interval(period);

        loop {
            select! {
                _ = ticker.tick() => self.check_health(),
                Some(v) = self.rx.recv() => self.handle_command(v),
            }
        }
    }

    fn handle_command(&mut self, cmd: Command<F>) {
        match cmd {
            Command::Create(cmd) => self.handle_create(cmd),
            Command::Delete(id) => self.handle_delete(id),
        }
    }

    fn handle_create(&mut self, cmd: CreateCommand<F>) {
        let _ = &self.state;
        // TODO: persist + insert in registry
    }

    fn handle_delete(&mut self, id: String) {
        let _ = &self.state;
        // TODO: remove from registry + cleanup state
    }

    fn check_health(&mut self) {
        let _ = &self.state;
        // TODO: periodic process health checks
    }
}
