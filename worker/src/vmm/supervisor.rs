use crate::vmm::Handle;
use futures::{StreamExt, stream::FuturesUnordered};
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
                _ = ticker.tick() => self.check_health().await,
                Some(v) = self.rx.recv() => self.handle_command(v).await,
            }
        }
    }

    async fn handle_command(&mut self, cmd: Command<F>) {
        match cmd {
            Command::Create(cmd) => self.handle_create(cmd).await,
            Command::Delete(id) => self.handle_delete(id).await,
        }
    }

    async fn handle_create(&mut self, cmd: CreateCommand<F>) {
        // Destructure the incoming command. Prefix variable names with `_` to
        // avoid unused-variable warnings until the implementation is added.
        let CreateCommand { id, handle } = cmd;

        if let Err(err) = handle.start().await {
            //TODO: let's do something more clever here and find how to keep track of what is needed to recreate the vm if we need to
            // maybe save the config in sqlite
            tracing::error!(id = %id, error = %err, "Failed to start VM");
            return;
        };

        //TODO: this takes a lock and releases it very quickly.
        // The vm should already be running at this point so we could have a buffer, keep them in the buffer and then save them all at once
        // Maybe this could be done with the channel instead of recv, we use recv_many
        self.state.insert(id, handle).await;
    }

    //TODO: maybe we want a stop function that can resume

    async fn handle_delete(&mut self, id: String) {
        if let Some(handle) = self.state.remove(&id).await {
            if let Err(err) = handle.delete().await {
                tracing::error!(id = %id, error = %err, "Failed to delete VM");
            }
            tracing::info!(id = %id, "VM deleted successfully");
        } else {
            tracing::warn!(id = %id, "Received delete command for non-existent VM");
        }
    }

    async fn check_health(&mut self) {
        // Grab a read guard to the registry. The guard must live while
        // we poll the health futures because those futures borrow the handles.
        let guard = self.state.clone().get().await;

        let mut futs: FuturesUnordered<_> = FuturesUnordered::new();

        for (id, handle) in guard.iter() {
            futs.push(async move { (id, handle.health().await) });
        }

        // Drive all health checks concurrently and log results as they complete.
        while let Some((id, res)) = futs.next().await {
            match res {
                Ok(()) => tracing::debug!(id = %id, "health OK"),
                //TODO: if there is an error we shouold do something. Maybe send to himself some message to delete or restart the vm?
                Err(e) => tracing::warn!(id = %id, error = %e, "health check failed"),
            }
        }
        // `guard` is dropped here
    }
}
