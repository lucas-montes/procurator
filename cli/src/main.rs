use cli::Cli;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    Cli::handle().unwrap_or_else(|err| {
        eprintln!("Error: {:?}", err);
        std::process::exit(1);
    });
}
