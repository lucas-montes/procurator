use std::sync::Arc;

use tokio::signal;
use tokio::sync::Notify;
use tracing::{error, info, warn};

/// Set up signal handlers for graceful shutdown.
pub fn setup_signal_handler() -> Arc<Notify> {
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        let mut sigint =
            unix::signal(unix::SignalKind::interrupt()).expect("failed to register SIGINT handler");
        let mut sigterm = unix::signal(unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        shutdown_clone.notify_one();
    });

    shutdown
}
