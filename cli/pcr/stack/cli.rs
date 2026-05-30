use clap::Args;
use std::path::PathBuf;

use super::commands::StackCommands;

#[derive(Debug, Args)]
pub struct StackArgs {
    /// Path to repo root (defaults to current directory)
    #[arg(short, long)]
    path: Option<PathBuf>,

    #[command(subcommand)]
    command: StackCommands,
}

impl StackArgs {
    pub fn execute(self) {
        let repo_path = self.path.unwrap_or_else(|| PathBuf::from("."));

        match self.command {
            StackCommands::Up => {
                if let Err(e) = super::parser::parse_and_run(&repo_path) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            StackCommands::Down => {
                println!("Bringing down the stack...");
            }
            StackCommands::Stop => {
                println!("Stopping the stack...");
            }
            StackCommands::Start => {
                println!("Starting the stack...");
            }
            StackCommands::Restart => {
                println!("Restarting the stack...");
            }
        }
    }
}

// `StackCommands` is defined in `commands.rs` and imported above.
