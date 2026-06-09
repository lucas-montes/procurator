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
    pub async fn execute(self) {
        let start_supervisor = |repo_root: PathBuf| {
            let state_repo = FileStackState::new(repo_root.clone());
            ProcessSupervisor::new(repo_root, state_repo)
        };

        match self.command {
            StackCommands::Start => {
                let graph =
                    parse_flake_services(&self.path).expect("unable to parse flake services");
                let supervisor = start_supervisor(self.path);
                supervisor.start_impl(&graph).await.expect("start failed");
            }
            StackCommands::Stop => {
                let mut supervisor = start_supervisor(self.path);
                supervisor.stop().expect("stack stop failed");
            }
        }
    }
}
