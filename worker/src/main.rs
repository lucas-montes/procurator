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

    worker::main(
        "hostname".into(),
        "127.0.0.1:6000".parse().expect("addr shold be valid"),
        "127.0.0.1:5000".parse().expect("addr shold be valid"),
    )
    .await;
}
