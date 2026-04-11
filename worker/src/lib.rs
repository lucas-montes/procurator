mod ch;
mod config;
mod server;
mod vmm;

use server::Server;
use tokio::join;
use tokio::task;

use crate::config::Config;

pub async fn main(config: Config) {
    // Server only holds the sending end — no VMM, no state
    let server = Server::new();

    // capnp-rpc requires spawn_local, which needs a LocalSet context
    let local_set = task::LocalSet::new();
    let server_task = local_set.run_until(task::spawn_local(server.serve(config.listen_addr)));

    match join!(manager_task, server_task) {
        (manager_result, server_result) => {
            if let Err(err) = manager_result {
                tracing::error!(?err, "Worker manager task panicked");
            }
            match server_result {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Worker server failed"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        }
    }
}
