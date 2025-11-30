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

use std::str::Chars;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{error, info};

use crate::error::WorkerError;
use crate::git_url;
use crate::queue::{Build, BuildQueue, BuildStatus};

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
                        error!(
                            build_id = build.id,
                            repo = build.repo_name,
                            branch = build.branch,
                            error = %e,
                            "Build processing failed"
                        );

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

        // Build git URL using our tested helper function
        let git_url = git_url::build_nix_git_url(&build.repo_path, &build.commit_hash)
            .map_err(|e| WorkerError::Git(format!("Invalid git URL: {}", e)))?;

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

        let output = Command::new("nix")
            .arg("flake")
            .arg("check")
            .arg(&git_url)
            .arg("--print-build-logs")
            .output()
            .await?;

        // Capture both stdout and stderr
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        info!(
            build_id = build.id,
            stdout_bytes = stdout.len(),
            stderr_bytes = stderr.len(),
            "Build command completed"
        );

        let full_logs = format!("{}{}{}", command_log, stdout, stderr);

        info!(
            build_id = build.id,
            total_log_bytes = full_logs.len(),
            "Storing build logs"
        );

        // Store the full logs
        self.queue
            .set_logs(build.id, &full_logs)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        if output.status.success() {
            info!(
                build_id = build.id,
                repo = build.repo_name,
                branch = build.branch,
                "Build succeeded"
            );
            self.queue
                .update_status(build.id, BuildStatus::Success)
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;
        } else {
            error!(
                build_id = build.id,
                repo = build.repo_name,
                branch = build.branch,
                exit_code = ?output.status.code(),
                stderr_preview = stderr.chars().take(500).collect::<String>().as_str(),
                "Build failed"
            );

            self.queue
                .update_status(build.id, BuildStatus::Failed)
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;

            return Err(WorkerError::Nix(stderr.to_string()));
        }

        Ok(())
    }
}
