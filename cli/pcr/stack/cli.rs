use clap::{Args, Subcommand};
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::stack::service::{ServiceManifest, Supervisor};
use crate::stack::watch::Watcher;

use super::config::{ServiceGraph, parse_stack_config};
use super::logging::{BothWriter, FileWriter, LogWriter, TerminalWriter};

#[derive(Debug, Subcommand)]
/// Subcommands for `pcr stack`
enum StackCommands {
    /// Start all services (foreground, Ctrl-C to stop)
    Start,
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
        match self.command {
            StackCommands::Start => {
                // ── Parse initial config ──────────────────────────────
                let (raw_services, log_config, watch_cfg) =
                    parse_stack_config(&self.path).expect("unable to parse flake config");
                let graph =
                    ServiceGraph::from_services(raw_services).expect("invalid service graph");
                let initial_manifest = ServiceManifest::from_graph(&graph, &self.path);

                // ── Channels ──────────────────────────────────────────
                // Channel 1: each service sends LogLine → log writer task
                let (logs_tx, logs_rx) = mpsc::channel(256);
                // Channel 2: watcher sends updated manifests → supervisor
                let (watcher_tx, watcher_rx) = mpsc::channel::<ServiceManifest>(16);

                // ── Task 1: Log writer ────────────────────────────────
                // Spawned background task: receives log lines from all
                // services and writes them to terminal (and optionally file).
                let log_task = if let Some(lc) = log_config {
                    let dir = if lc.dir().is_relative() {
                        self.path.join(lc.dir())
                    } else {
                        lc.dir().to_path_buf()
                    };
                    let file = FileWriter::new_file(&dir).expect("could not create log file");
                    let filew = FileWriter::new(dir, lc.max_lines(), file);
                    BothWriter::new(TerminalWriter::default(), filew).spawn(256, logs_rx)
                } else {
                    TerminalWriter::default().spawn(256, logs_rx)
                };

                // ── Task 2: File watcher ──────────────────────────────
                // Spawned background task (only if enabled): monitors the
                // repo for flake changes, re-parses config, and sends a
                // fresh ServiceManifest on the channel.

                let watcher_task = if let Some(wc) = watch_cfg {
                    if wc.enabled() {
                        Some(Watcher::new(watcher_tx, self.path).spawn(256))
                    } else {
                        None
                    }
                } else {
                    None
                };

                // ── Task 3: Supervisor (foreground) ───────────────────
                // Blocks until Ctrl-C: spawns all services in dependency
                // order, then listens for new manifests from the watcher.
                // On each manifest it diffs and restarts affected services.
                let supervisor = Supervisor::new(initial_manifest, logs_tx, watcher_rx);
                supervisor.run().await.expect("supervisor failed");

                // ── Shutdown ───────────────────────────────────────────
                // Supervisor has killed all services. Cancel the watcher,
                // then wait for the log writer to drain remaining lines.
                if let Some(task) = watcher_task {
                    task.abort();
                }
                log_task.await.expect("log task failed");
            }
        }
    }
}
