mod ch;
mod config;
mod database;
mod server;
mod vmm;

use std::path::Path;
use tokio::sync::mpsc;

use server::Server;
use tokio::join;
use tokio::task;

use config::Config;
use tracing::debug;
use vmm::{Factory, Registry, Supervisor};

pub async fn ch_main(path: impl AsRef<Path> + std::fmt::Debug) {
    main::<ch::Factory>(path).await;
}

async fn main<F>(path: impl AsRef<Path> + std::fmt::Debug)
where
    F: Factory,
{
    let config: Config<F> = Config::from_file(path);
    debug!(?config, "Loaded worker configuration");

    let db = database::Database::new("sqlite::memory:").await;

    let registry: Registry<F, _> = Registry::new(db);
    let (reader_registry, writer_registry) = registry.split();

    let (tx, rx) = mpsc::channel(100);

    let factory = F::from(config.vmm);

    let server = Server::new(reader_registry, factory, tx, config.listen_addr);
    let supervisor = Supervisor::new(writer_registry, rx);

    let local_set = task::LocalSet::new();
    let server_task = local_set
        .run_until(async move { task::spawn_local(server.serve(config.listen_addr)).await });
    let supervisor_task = task::spawn(supervisor.run(config.health_tick_millis));

    let (supervisor_result, server_result) = join!(supervisor_task, server_task);

    if let Err(err) = supervisor_result {
        tracing::error!(?err, "Worker supervisor task panicked");
    }

    match server_result {
        Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
        Ok(Err(err)) => tracing::error!(?err, "Worker server failed"),
        Err(err) => tracing::error!(?err, "Worker server task panicked"),
    }
}
