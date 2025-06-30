mod client;
mod commands;
mod interactive;

use commands::Cli;


#[tokio::main(flavor = "current_thread")]
async fn main() {
    Cli::handle().await.unwrap_or_else(|err| {
        eprintln!("Error: {:?}", err);
        std::process::exit(1);
    });
}
