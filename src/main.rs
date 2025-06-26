use cli::Cli;

fn main() {
    Cli::handle().unwrap_or_else(|err| {
        eprintln!("Error: {:?}", err);
        std::process::exit(1);
    });
}
