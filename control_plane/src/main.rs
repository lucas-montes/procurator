use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let Some(config_path) = config_path else {
        eprintln!("Config path must be provided as the first argument");
        std::process::exit(1);
    };

    // Logging first, from the config's `log` section, so every later line
    // (including config errors) uses the configured format and destination.
    let _log_guard = control_plane::logging::init(&config_path, "info");

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
        std::time::Duration::from_millis(config.state_log_interval_millis.get()),
    )
    .await;
}
