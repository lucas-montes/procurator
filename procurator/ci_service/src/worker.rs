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


trait SomeTrait {
    const
}

use std::sync::Arc;
use tracing::{error, info};

use crate::config::Config;
use crate::nix_parser::run_checks_with_logs;
use crate::queue::{Build, BuildQueue, BuildStatus};

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

impl From<crate::nix_parser::checks::ChecksError> for WorkerError {
    fn from(err: crate::nix_parser::checks::ChecksError) -> Self {
        match err {
            crate::nix_parser::checks::ChecksError::IoError(io_err) => WorkerError::Io(io_err),
            crate::nix_parser::checks::ChecksError::ProcessFailed {
                exit_code: _,
                stderr,
            } => WorkerError::Nix(stderr),
            crate::nix_parser::checks::ChecksError::JsonParseError(json_err) => {
                WorkerError::Process(format!("JSON parse error: {}", json_err))
            }
            crate::nix_parser::checks::ChecksError::InvalidFlakePath(path) => {
                WorkerError::Nix(format!("Invalid flake path: {}", path))
            }
            crate::nix_parser::checks::ChecksError::Timeout(duration) => {
                WorkerError::Process(format!("Process timed out after {:?}", duration))
            }
        }
    }
}

type Result<T> = std::result::Result<T, WorkerError>;

pub struct Worker {
    queue: Arc<BuildQueue>,
}

impl Worker {
    pub fn new(queue: Arc<BuildQueue>) -> Self {
        Self { queue }
    }

    pub async fn run(&self) -> Result<()> {
        info!(target: "procurator::worker", "Worker started");

        loop {
            match self.queue.get_pending().await {
                Ok(Some(build)) => {
                    if let Err(e) = self.process_build(&build).await {
                        if let WorkerError::Queue(err) = e {
                            error!(
                                build_id = build.id,
                                repo = build.repo_name,
                                branch = build.branch,
                                error = %err,
                                "Build processing failed"
                            );
                        }

                        // Check if we can retry
                        if self.queue.can_retry(&build).await {
                            info!(
                                build_id = build.id,
                                attempt = build.retry_count + 2,
                                max_retries = build.max_retries + 1,
                                "Scheduling retry for build"
                            );
                            self.queue
                                .increment_retry(build.id)
                                .await
                                .map_err(|e| WorkerError::Database(e.to_string()))?;
                        } else {
                            info!(
                                build_id = build.id,
                                max_retries = build.max_retries + 1,
                                "Build exhausted all retries, marking as failed"
                            );
                            self.queue
                                .update_status(build.id, BuildStatus::Failed)
                                .await
                                .map_err(|e| WorkerError::Database(e.to_string()))?;
                        }
                    }
                }
                Ok(None) => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    error!(error = %e, "Error fetching pending builds");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    //TODO: we need to ensure that the repo has a flake.nix
    async fn process_build(&self, build: &Build) -> Result<()> {
        info!(
            build_id = build.id,
            repo = build.repo_name,
            branch = build.branch,
            commit = build.commit_hash,
            attempt = build.retry_count + 1,
            max_retries = build.max_retries + 1,
            "Starting build processing"
        );

        self.queue
            .update_status(build.id, BuildStatus::Running)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        // Parse repo path: /base/username/repo.git
        let config = Config::init();
        let repo_path = build
            .repo_path(&config.repos_base_path)
            .map_err(|e| WorkerError::Queue(e.to_string()))?;

        let git_url = repo_path.to_nix_url_with_rev(&build.commit_hash);

        info!(
            build_id = build.id,
            git_url = git_url.as_str(),
            "Executing nix flake check"
        );

        // Store the command in logs
        let command_log = format!("$ nix flake check {} --print-build-logs\n", git_url);
        self.queue
            .set_logs(build.id, &command_log)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        match run_checks_with_logs(&git_url).await {
            Ok(summary) => {
                // Store the structured build summary
                self.queue
                    .set_build_summary(build.id, &summary)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                info!(
                    build_id = build.id,
                    repo = build.repo_name,
                    branch = build.branch,
                    duration = ?summary.total_duration(),
                    steps_count = summary.steps().len(),
                    packages_checked = summary.packages_checked().len(),
                    checks_run = summary.checks_run().len(),
                    "Build completed successfully"
                );

                self.queue
                    .update_status(build.id, BuildStatus::Success)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;
            }
            Err(e) => {
                error!(
                    build_id = build.id,
                    repo = build.repo_name,
                    branch = build.branch,
                    error = %e,
                    "Build failed"
                );

                // Store the error information in logs
                let error_log = format!("Build failed: {}\n", e);
                self.queue
                    .append_log(build.id, &error_log)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                self.queue
                    .update_status(build.id, BuildStatus::Failed)
                    .await
                    .map_err(|e| WorkerError::Database(e.to_string()))?;

                return Err(WorkerError::Nix(format!("Build failed: {}", e)));
            }
        }

        Ok(())
    }
}
