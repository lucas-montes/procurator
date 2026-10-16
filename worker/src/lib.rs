mod ch;
mod config;
mod database;
mod heartbeat;
mod proxy;
mod pull;
mod server;
mod vmm;

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod test_support;

use std::path::Path;
use tokio::sync::mpsc;

use proxy::serve_tls_proxy;
use server::Server;
use tokio::select;
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

    run(
        config.rpc_listen_addr,
        config.proxy,
        config.health_tick_millis,
        pull::PollConfig {
            worker_id: config.worker_id,
            master_addr: config.master_addr,
            poll_interval: std::time::Duration::from_millis(config.poll_interval_millis.get()),
            heartbeat_interval: std::time::Duration::from_millis(
                config.heartbeat_interval_millis.get(),
            ),
        },
        factory,
        db,
    )
    .await;
}

async fn run<F>(
    rpc_listen_addr: std::net::SocketAddr,
    proxy_config: config::ProxyConfig,
    health_tick_millis: std::num::NonZeroU64,
    poll_config: pull::PollConfig,
    factory: F,
    db: database::Database,
) where
    F: Factory,
    F::BackendConfig: Unpin,
{
    let registry: Registry<F, _> = Registry::new(db);
    let (reader_registry, writer_registry) = registry.split();

    let (tx, rx) = mpsc::channel(100);

    let reconciler = pull::Reconciler::new(factory.clone(), tx.clone());
    let server = Server::new(reader_registry.clone(), factory, tx, rpc_listen_addr);
    let supervisor = Supervisor::new(writer_registry, rx);

    let supervisor_task = task::spawn(supervisor.run(health_tick_millis));
    let proxy_task = task::spawn(serve_tls_proxy(proxy_config, reader_registry.clone()));

    // Poll and heartbeat share the applied-generation cell: the heartbeat
    // reports whatever the poll loop last reconciled (0 until then).
    let applied = pull::AppliedGeneration::new();

    let local_set = task::LocalSet::new();
    let rpc_handle = local_set.spawn_local(server.serve(rpc_listen_addr));
    let poll_handle = local_set.spawn_local(pull::run_poll_loop(
        poll_config.clone(),
        reconciler,
        reader_registry.clone(),
        applied.clone(),
    ));
    let heartbeat_handle = local_set.spawn_local(heartbeat::run_heartbeat_loop(
        poll_config,
        reader_registry,
        applied,
    ));

    select! {
        res = supervisor_task => tracing::error!(?res, "supervisor exited"),
        res = proxy_task      => tracing::error!(?res, "proxy exited"),
        res = local_set.run_until(async {
            let _ = tokio::join!(rpc_handle, poll_handle, heartbeat_handle);
        }) => tracing::error!(?res, "local tasks exited"),
    }
}
