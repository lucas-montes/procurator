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

use tracing::debug;
use vmm::{Factory, Registry, Supervisor};

//TODO: not a fan of this refactor made by the AI but it works
pub async fn ch_main(path: impl AsRef<Path> + std::fmt::Debug) {
    let config: config::Config<ch::Factory> = config::Config::from_file(&path);
    debug!(?config, "Loaded worker configuration");

    let db_url = format!("{}{}", "sqlite:", config.vmm.state_db_path().display());

    tracing::debug!(db_url=%db_url, "Constructed database URL");
    let db: database::Database = database::Database::new(&db_url).await;

    let factory = ch::Factory::new(config.vmm, db.clone());

    run(config.listen_addr, config.health_tick_millis, factory, db).await;
}

async fn run<F: Factory>(
    listen_addr: std::net::SocketAddr,
    health_tick_millis: std::num::NonZeroU64,
    factory: F,
    db: database::Database,
) {
    let registry: Registry<F, _> = Registry::new(db);
    let (reader_registry, writer_registry) = registry.split();

    let (tx, rx) = mpsc::channel(100);

    let server = Server::new(reader_registry, factory, tx, listen_addr);
    let supervisor = Supervisor::new(writer_registry, rx);

    let local_set = task::LocalSet::new();
    let server_task =
        local_set.run_until(async move { task::spawn_local(server.serve(listen_addr)).await });
    let supervisor_task = task::spawn(supervisor.run(health_tick_millis));

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
