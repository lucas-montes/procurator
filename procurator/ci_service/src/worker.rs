use std::sync::Arc;
use tokio::process::Command;
use tracing::{error, info};

use crate::error::WorkerError;
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
        info!("Worker started");

        loop {
            match self.queue.get_pending().await {
                Ok(Some(build)) => {
                    if let Err(e) = self.process_build(&build).await {
                        error!("Build #{} failed: {}", build.id, e);

                        // Check if we can retry
                        if self.queue.can_retry(&build).await {
                            info!(
                                "Retrying build #{} (attempt {}/{})",
                                build.id,
                                build.retry_count + 2,
                                build.max_retries + 1
                            );
                            self.queue
                                .increment_retry(build.id)
                                .await
                                .map_err(|e| WorkerError::Database(e.to_string()))?;
                        } else {
                            info!(
                                "Build #{} exhausted all retries, marking as failed",
                                build.id
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
                    error!("Error fetching builds: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn process_build(&self, build: &Build) -> Result<()> {
        info!(
            "Processing build #{}: {}/{} (attempt {}/{})",
            build.id,
            build.repo,
            build.branch,
            build.retry_count + 1,
            build.max_retries + 1
        );

        self.queue
            .update_status(build.id, BuildStatus::Running)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        // Build git URL from bare repo path and commit hash
        // This allows Nix to fetch directly from the bare repo without cloning
        let git_url = format!("git+file://{}?rev={}", build.repo, build.commit_hash);

        info!("Running: nix flake check {}", git_url);

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
        let full_logs = format!("{}{}{}", command_log, stdout, stderr);

        // Store the full logs
        self.queue
            .set_logs(build.id, &full_logs)
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        if output.status.success() {
            info!("Build #{} succeeded", build.id);
            self.queue
                .update_status(build.id, BuildStatus::Success)
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;
        } else {
            error!("Build #{} failed:\n{}", build.id, stderr);

            self.queue
                .update_status(build.id, BuildStatus::Failed)
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;

            return Err(WorkerError::Nix(stderr.to_string()));
        }

        Ok(())
    }
}
