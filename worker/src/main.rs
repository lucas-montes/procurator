use std::path::PathBuf;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use worker::Config;

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "info,hyper=warn,h2=warn,tower=warn,capnp_rpc=warn",
            )
        });

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .log_internal_errors(true)
                .with_target(false),
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

    worker::main(cfg)
    .await;
}
