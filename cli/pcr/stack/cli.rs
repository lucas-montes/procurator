use clap::{Args, Subcommand};
use std::path::PathBuf;

use tokio::sync::mpsc;

use super::parser::parse_flake_services;
use super::process::{LogLine, ProcessSupervisor, file_writer};
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
                let (graph, log_config) =
                    parse_flake_services(&self.path).expect("unable to parse flake services");

                let mut supervisor = start_supervisor(self.path.clone());
                let mut _log_handle = None;

                // If file logging is configured, set up the channel and writer task.
                if let Some(lc) = log_config {
                    let dir = if lc.dir.is_relative() {
                        self.path.join(&lc.dir)
                    } else {
                        lc.dir
                    };
                    let (tx, rx) = mpsc::channel::<LogLine>(256);
                    supervisor.log_sender = Some(tx);
                    _log_handle = Some(tokio::spawn(file_writer(rx, dir, lc.max_lines)));
                }

                supervisor.start_impl(&graph).await.expect("start failed");

                // Drop supervisor to close our end of the log channel.
                // The reader tasks will detect EOF from dead children, exit,
                // drop their sender clones, and the file writer will finish.
                drop(supervisor);

                if let Some(handle) = _log_handle {
                    let _ = handle.await;
                }
            }
            StackCommands::Stop => {
                let mut supervisor = start_supervisor(self.path);
                supervisor.stop().expect("stack stop failed");
            }
        }
    }
}
