//! Procurator console: a read-only web UI that shows the cluster live by
//! following each service's own logs (see `context/specs/log-events.md`).

mod config;
mod http;
mod model;
mod parse;
mod sources;

use std::path::PathBuf;

use tokio::sync::mpsc;

use config::{Config, SourceKind};
use sources::{follow_file, follow_gateway, follow_journalctl};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(path) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: console <config.json>");
        std::process::exit(2);
    };
    let config = Config::from_file(&path).unwrap_or_else(|err| {
        eprintln!("{err}");
        std::process::exit(1);
    });

    let http_addr = config.http_addr();
    let entries = config.bootstrap_entries();
    let (model, channels) = model::Model::new(config.sources(), config.stale_after_millis());

    let (events_tx, events_rx) = mpsc::channel(4096);
    for (index, source) in config.into_sources().into_iter().enumerate() {
        let tx = events_tx.clone();
        tracing::info!(name = source.name(), kind = %source.kind(), "following source");
        match source.into_kind() {
            SourceKind::File { path } => {
                tokio::spawn(follow_file(index, path, entries, tx));
            }
            SourceKind::JournalGateway { url, unit } => {
                tokio::spawn(follow_gateway(index, url, unit, entries, tx));
            }
            SourceKind::JournalLocal { unit } => {
                tokio::spawn(follow_journalctl(index, unit, entries, tx));
            }
        }
    }
    drop(events_tx);

    tokio::spawn(model.run(events_rx));
    let (snapshot, messages, logs) = channels.into_parts();

    if let Err(err) = http::serve(http_addr, snapshot, messages, logs).await {
        tracing::error!(%err, "console HTTP server failed");
        std::process::exit(1);
    }
}
