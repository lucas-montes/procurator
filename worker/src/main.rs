use std::path::PathBuf;

use tracing_subscriber::{fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "debug,sqlx=warn,hyper=warn,h2=warn,tower=warn,capnp_rpc=warn",
        )
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .log_internal_errors(true)
                .with_level(true)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .with_span_events(FmtSpan::CLOSE),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("Config path must be provided as the first argument");

    worker::ch_main(config_path).await;
}
