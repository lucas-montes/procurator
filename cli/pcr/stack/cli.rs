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
    #[arg(short, long, default_value = ".")]
    path: PathBuf,

    #[command(subcommand)]
    command: StackCommands,
}

impl StackArgs {
    pub fn execute(self) {
        let start_supervisor = |repo_root: PathBuf| {
            let state_repo = FileStackState::new(repo_root.clone());
            ProcessSupervisor::new(repo_root, state_repo)
        };

        match self.command {
            StackCommands::Start => {
                let graph = match parse_flake_services(&self.path) {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                };
                let mut supervisor = start_supervisor(self.path);
                if let Err(e) = supervisor.start(&graph) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            StackCommands::Stop => {
                let mut supervisor = start_supervisor(self.path);
                if let Err(e) = supervisor.stop() {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
