pub mod ch;
pub mod config;
pub mod database;
pub mod proxy;
pub mod server;
pub mod signal_handler;
pub mod vmm;

use std::path::Path;
use tokio::sync::mpsc;

use server::Server;
use tokio::join;
use tokio::task;

use tracing::{debug, info};
use vmm::{Factory, Registry, Supervisor};

use proxy::start_proxy_server;
use signal_handler::setup_signal_handler;

//TODO: not a fan of this refactor made by the AI but it works
pub async fn ch_main(path: impl AsRef<Path> + std::fmt::Debug) {
    let config: config::Config<ch::Factory> = config::Config::from_file(&path);
    debug!(?config, "Loaded worker configuration");

    let db_url = format!("{}:{}", "sqlite:", config.vmm.state_db_path().display());

    tracing::debug!(db_url=%db_url, "Constructed database URL");
    let db: database::Database = database::Database::new(&db_url).await;

    let factory = ch::Factory::new(config.vmm, db.clone());

    run_worker(
        config.listen_addr,
        config.health_tick_millis,
        factory,
        db,
        config.proxy,
    )
    .await;
}

async fn run_worker<F: Factory>(
    listen_addr: std::net::SocketAddr,
    health_tick_millis: std::num::NonZeroU64,
    factory: F,
    db: database::Database,
    proxy_config: config::ProxyConfig,
) {
    let registry: Registry<F, _> = Registry::new(db.clone());
    let (reader_registry, writer_registry) = registry.split();

    let (tx, rx) = mpsc::channel(100);

    // Clone proxy_config before moving it into Server::new
    let proxy_config_for_server = proxy_config.clone();
    let server = Server::new(
        reader_registry.clone(),
        factory,
        tx,
        listen_addr,
        proxy_config_for_server,
    );
    let supervisor = Supervisor::new(writer_registry, rx);

    // Spawn proxy server task
    let proxy_db = db.clone();
    let _proxy_listen_addr = proxy_config.listen_addr;
    let _proxy_enable_tls = proxy_config.enable_tls;
    let proxy_cert_path = proxy_config.tls_cert_path.clone();
    let proxy_key_path = proxy_config.tls_key_path.clone();

    let proxy_task = task::spawn(async move {
        if let Err(e) = start_proxy_server(
            _proxy_listen_addr,
            proxy_db,
            _proxy_enable_tls,
            proxy_cert_path,
            proxy_key_path,
        )
        .await
        {
            tracing::error!(?e, "Proxy server failed");
        }
    });

    // Setup signal handler for graceful shutdown
    let shutdown = setup_signal_handler();

    // Spawn server and supervisor tasks
    let local_set = task::LocalSet::new();
    let server_task =
        local_set.run_until(async move { task::spawn_local(server.serve(listen_addr)).await });
    let supervisor_task = task::spawn(supervisor.run(health_tick_millis));

    // Wait for either a shutdown signal or task completion
    tokio::select! {
        _ = shutdown.notified() => {
            info!("Shutdown signal received, cleaning up...");
            // Give tasks a moment to finish ongoing work
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
        (supervisor_result, server_result, _) =
            join!(supervisor_task, server_task, proxy_task) => {
            if let Err(err) = supervisor_result {
                tracing::error!(?err, "Worker supervisor task panicked");
            }

            match server_result {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Worker server failed"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        }
    }

    // Cleanup: delete TAP interfaces and kill remaining VMs
    info!("Starting cleanup...");
    let registry = Registry::new(db.clone());
    let handles: Vec<_> = registry.get().await.into_values().collect();
    for (vm_id, handle) in handles {
        info!(%vm_id, "Cleaning up VM on shutdown");
        if let Err(e) = handle.shutdown().await {
            tracing::error!(%vm_id, ?e, "Failed to cleanup VM during shutdown");
        }
    }

    info!("Worker shut down complete");
}
