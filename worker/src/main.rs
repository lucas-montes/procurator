use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

    worker::main(
        "hostname".into(),
        "127.0.0.1:6000".parse().expect("addr shold be valid"),
        "127.0.0.1:5000".parse().expect("addr shold be valid"),
    )
    .await;
}
