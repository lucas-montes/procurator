use std::sync::mpsc::Receiver;

use super::{interfaces::Factory, registry::{Registry, Writer}};


pub enum Command {
    Create,
    Delete,
}


/// Background structure that will save processes running, check their status, and make operations to the state if needed. It will be the primary writter for blocking operations
pub struct Supervisor<F: Factory> {
    state: Registry<F, Writer>,
    rx: Receiver<Command>,
}

impl<F: Factory> Supervisor<F> {
    pub fn new(state: Registry<F, Writer>, rx: Receiver<Command>) -> Self {
        Self { state, rx }
    }

    pub fn run(self) {
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                Command::Create => {
                    let _ = &self.state;
                }
                Command::Delete => {
                    let _ = &self.state;
                }
            }
        }
    }
}
