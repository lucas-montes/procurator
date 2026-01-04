//! CI Build Worker
//!
//! Polls the build queue for pending builds and executes them using Nix.
//! This module handles:
//! - Polling the queue for pending builds
//! - Running `nix flake check` on the build target
//! - Capturing build output (stdout/stderr) and storing logs
//! - Updating build status in the queue (Queued → Running → Success/Failed)
//! - Implementing retry logic with exponential backoff
//!
//! The worker runs in a background task and continuously polls the queue
//! at configurable intervals, processing builds serially.

use repo_outils::nix;
use std::sync::Arc;
use tracing::{error, info};

use crate::builds::{BuildJob, BuildStatus};
use crate::database::BuildSummary as DbBuildSummary;
use crate::job_queue::JobQueue;

use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum WorkerError {
    Database(String),
    Process(String),
    Nix(String),
    Git(String),
    Io(std::io::Error),
    Queue(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::Database(msg) => write!(f, "Database error: {}", msg),
            WorkerError::Process(msg) => write!(f, "Process error: {}", msg),
            WorkerError::Nix(msg) => write!(f, "Nix build error: {}", msg),
            WorkerError::Git(msg) => write!(f, "Git error: {}", msg),
            WorkerError::Io(err) => write!(f, "IO error: {}", err),
            WorkerError::Queue(msg) => write!(f, "Queue error: {}", msg),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WorkerError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for WorkerError {
    fn from(err: std::io::Error) -> Self {
        WorkerError::Io(err)
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for WorkerError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        WorkerError::Database(err.to_string())
    }
}

// Update this to use the new nix::Error instead of nix::LogsError
impl From<nix::Error> for WorkerError {
    fn from(err: nix::Error) -> Self {
        match err {
            nix::Error::Io(io_err) => WorkerError::Io(io_err),
            nix::Error::ProcessFailed {
                exit_code: _,
                stderr,
            } => WorkerError::Nix(stderr),
            nix::Error::JsonParse(json_err) => {
                WorkerError::Process(format!("JSON parse error: {}", json_err))
            }
            nix::Error::InvalidFlakePath(path) => {
                WorkerError::Nix(format!("Invalid flake path: {}", path))
            }
            nix::Error::LogParsing(log_err) => {
                WorkerError::Process(format!("Log parsing error: {}", log_err))
            }
            nix::Error::BuildOutputMissing => {
                WorkerError::Process("Build output missing".to_string())
            }
        }
    }
}

type Result<T> = std::result::Result<T, WorkerError>;

pub struct Worker {
    queue: JobQueue,
}

impl Worker {
    pub fn new(queue: JobQueue) -> Self {
        Self { queue }
    }

    pub async fn run(self) {
        info!(target: "ci_service::worker", "Worker started");

        loop {
            match self.queue.get_pending().await {
                Ok(Some(build)) => {
                    if let Err(err) = self.process_build(&build).await {
                        error!(
                            ?build,
                            %err,
                            "Build failed"
                        );

                        // Check if we can retry
                        if build.can_retry() {
                            info!(?build, "Scheduling retry for build");

                            if let Err(err) = self
                                .queue
                                .increment_retry(build.id())
                                .await
                                .map_err(|e| WorkerError::Database(e.to_string()))
                            {
                                error!(
                                    build_id = build.id(),
                                    %err,
                                    "Failed to increment retry count"
                                );
                            };
                        } else {
                            info!(
                                build_id = build.id(),
                                "Build exhausted all retries, marking as failed"
                            );
                            if let Err(err) = self
                                .queue
                                .update_status(build.id(), BuildStatus::Failed)
                                .await
                                .map_err(|e| WorkerError::Database(e.to_string()))
                            {
                                error!(
                                    build_id = build.id(),
                                     %err,
                                    "Failed to update build status to Failed"
                                );
                            };
                        }
                    }
                }
                Ok(None) => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                }
                Err(err) => {
                    error!(%err, "Error fetching pending builds");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    //TODO: we need to ensure that the repo has a flake.nix
    async fn process_build(&self, build: &BuildJob) -> Result<()> {
        info!(
            build = ?build,
            "Starting build processing"
        );

        self.queue
            .update_status(build.id(), BuildStatus::Running)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        let git_url = build.git_url();

        info!(build_id = build.id(), git_url, "Executing nix flake check");

        // Store the command in logs
        // TODO: this can be a reason why we want a struct NixCommand, to store the actual command used
        let command_log = format!("$ nix flake check {} --print-build-logs\n", git_url);
        self.queue
            .set_logs(build.id(), &command_log)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        // Use the new flake_check function that returns CheckResult
        match nix::flake_check(&git_url).await {
            Ok(check_result) => {
                // Access the summary through the accessor method
                // let summary = check_result.summary();

                // Convert repo_outils::nix::Summary into your database::BuildSummary
                // You'll need to implement this conversion based on your DbBuildSummary structure
                let db_summary = DbBuildSummary;

                // Store the structured build summary
                self.queue
                    .set_build_summary(build.id(), &db_summary)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                self.queue
                    .update_status(build.id(), BuildStatus::Success)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                // info!(
                //     build_id = build.id(),
                //     total_steps = summary.total_steps,
                //     "Build completed successfully"
                // );
            }
            Err(e) => {
                error!(
                    build_id = build.id(),
                    git_url,
                    error = %e,
                    "Build failed"
                );

                // Store the error information in logs
                let error_log = format!("Build failed: {}\n", e);
                self.queue
                    .append_log(build.id(), &error_log)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                self.queue
                    .update_status(build.id(), BuildStatus::Failed)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                return Err(WorkerError::Nix(format!("Build failed: {}", e)));
            }
        }

        Ok(())
    }
}
