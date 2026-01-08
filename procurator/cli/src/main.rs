// mod client;
// mod interactive;
mod cli;
mod init;
mod autonix;

use cli::Cli;


#[tokio::main(flavor = "current_thread")]
async fn main() {
    Cli::handle().await.unwrap_or_else(|err| {
        eprintln!("Error: {:?}", err);
        std::process::exit(1);
    });
}
