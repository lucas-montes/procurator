use clap::{Args, Subcommand};
use std::path::PathBuf;

use super::parser::parse_flake_services;
use super::process::ProcessSupervisor;
use super::supervisor::{FileStackState, ServiceSupervisor};

#[derive(Debug, Subcommand)]
/// Subcommands for `pcr stack`
enum StackCommands {
    /// Start all services (foreground, Ctrl-C to stop)
    Start,
    /// Stop all running services (cross-terminal)
    Stop,
}

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
            StackCommands::Start => {
                let graph = match parse_flake_services(&repo_path) {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                };
                let state_repo = FileStackState::new(repo_path.clone());
                let mut supervisor = ProcessSupervisor::new(repo_path, Box::new(state_repo));
                if let Err(e) = supervisor.start(&graph) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            StackCommands::Stop => {
                let state_repo = FileStackState::new(repo_path.clone());
                let mut supervisor = ProcessSupervisor::new(repo_path, Box::new(state_repo));
                if let Err(e) = supervisor.stop() {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
