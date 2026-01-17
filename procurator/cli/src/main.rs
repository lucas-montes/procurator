// mod client;
// mod interactive;
mod cli;
mod init;

use cli::Cli;


#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
    .with_env_filter("info")
    .init();
    Cli::handle().await.unwrap_or_else(|err| {
        tracing::error!(?err, "Error");
        std::process::exit(1);
    });
}
