use std::{net::SocketAddr, path::PathBuf};

use serde::Deserialize;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Role {
    Master { peers_addr: Vec<SocketAddr> },
    Worker { master_addr: SocketAddr },
}

#[derive(Debug, Deserialize)]
struct Config {
    hostname: String,
    addr: SocketAddr,
    role: Role,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                // .with_writer(non_blocking)
                .log_internal_errors(true)
                .with_target(false)
                .flatten_event(true)
                .with_span_list(false),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("Config path must be provided as the first argument");

    let contents = tokio::fs::read(&config_path).await.unwrap_or_else(|e| {
        tracing::error!(path = ?config_path, error = %e, "Could not read config");
        std::process::exit(1);
    });

    let cfg: Config = serde_json::from_slice(&contents).unwrap_or_else(|e| {
        tracing::error!(path = ?config_path, error = %e, "Failed to parse config");
        std::process::exit(1);
    });

    tracing::info!(path = ?config_path, ?cfg, "Loaded configuration");

    match cfg.role {
        Role::Master { peers_addr } => {
            tracing::info!(?peers_addr, "Starting in Master mode");
            control_plane::main(cfg.hostname, cfg.addr, peers_addr).await;
        }
        Role::Worker { master_addr } => {
            tracing::info!(?master_addr, "Starting in Worker mode");
            worker::main(cfg.hostname, cfg.addr, master_addr).await;
        }
    }
}
