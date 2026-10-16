use std::path::PathBuf;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let Some(config_path) = config_path else {
        tracing::error!("Config path must be provided as the first argument");
        std::process::exit(1);
    };

    // Fail-fast: unreadable, unparsable, or invalid inventory (duplicate or
    // blank worker ids) exits here. Peer discovery from the `role`
    // envelope stays dormant until HA work lands.
    let config = control_plane::config::Config::from_file(&config_path);
    let declared = config.declared_workers().unwrap_or_else(|err| {
        tracing::error!(%err, "Invalid worker inventory");
        std::process::exit(1);
    });
    tracing::info!(
        hostname = %config.hostname,
        addr = %config.addr,
        workers = config.workers.len(),
        path = ?config_path,
        "Loaded control plane configuration",
    );

    control_plane::main::<capnp::any_pointer::Owned>(
        config.hostname,
        config.addr,
        Vec::new(),
        declared,
    )
    .await;
}
