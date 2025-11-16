use std::sync::Arc;
use tokio::process::Command;
use tracing::{info, error};

use crate::queue::BuildQueue;
use crate::error::WorkerError;

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
                    info!("Processing build #{}: {}/{}", build.id, build.repo, build.branch);

                    if let Err(e) = self.process_build(&build).await {
                        error!("Build #{} failed: {}", build.id, e);
                        self.queue.update_status(build.id, "failed").await?;
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

    async fn process_build(&self, build: &crate::queue::Build) -> Result<()> {
        self.queue
            .update_status(build.id, "running")
            .await
            .map_err(|e| WorkerError::Database(e.to_string()))?;

        // Run nix flake check in the repo
        let repo_path = format!("../{}", build.repo);

        info!("Running: nix flake check in {}", repo_path);

        let output = Command::new("nix")
            .arg("flake")
            .arg("check")
            .arg(&repo_path)
            .arg("--print-build-logs")
            .output()
            .await?;

        if output.status.success() {
            info!("Build #{} succeeded", build.id);
            self.queue
                .update_status(build.id, "success")
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Build #{} failed:\n{}", build.id, stderr);

            self.queue
                .update_status(build.id, "failed")
                .await
                .map_err(|e| WorkerError::Database(e.to_string()))?;

            return Err(WorkerError::Nix(stderr.to_string()));
        }

        Ok(())
    }
}
