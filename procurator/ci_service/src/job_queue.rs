//! Job Queue Management
//!
//! Manages the lifecycle of CI build jobs.
//! Provides operations for:
//! - Enqueuing new builds from Git hooks
//! - Polling pending builds for the worker
//! - Updating build status (Queued → Running → Success/Failed)
//! - Storing build logs and metadata
//! - Retry logic with configurable maximum attempts
//!
//! The queue is thread-safe and can be shared across multiple tasks via Arc.

use std::path::PathBuf;

use crate::database::{Database, DatabaseError};
use crate::domain::{Build, BuildStatus};
use crate::repo_manager::RepositoryStore;
use crate::nix_parser::checks::BuildSummary;

pub type Result<T> = std::result::Result<T, DatabaseError>;

#[derive(Clone)]
pub struct JobQueue {
    db: Database,
    repo_store: RepositoryStore,
}

impl JobQueue {
    pub fn new(db: Database) -> Self {
        let repo_store = RepositoryStore::new(db.clone());
        Self { db, repo_store }
    }

    /// Enqueue a new build job
    pub async fn enqueue(&self, repo_path: PathBuf, commit: &str, branch: &str) -> Result<i64> {
        let repo_path_str = repo_path.to_string_lossy().to_string();
        let repo_id = self.repo_store.get_or_create_repo(&repo_path_str).await?;

        let result = sqlx::query(
            "INSERT INTO builds (repo_id, commit_hash, branch) VALUES (?, ?, ?)"
        )
        .bind(repo_id)
        .bind(commit)
        .bind(branch)
        .execute(self.db.pool())
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get the next pending build from the queue
    pub async fn get_pending(&self) -> Result<Option<Build>> {
        let build_row = sqlx::query_as::<_, crate::database::BuildRow>(
            r#"
            SELECT
                b.id, b.commit_hash, b.branch, b.status,
                b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at,
                r.name as repo_name,
                u.username as username
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            JOIN users u ON r.user_id = u.id
            WHERE b.status = ?
            ORDER BY b.created_at
            LIMIT 1
            "#,
        )
        .bind(BuildStatus::Queued.as_str())
        .fetch_optional(self.db.pool())
        .await?;

        Ok(build_row.map(Build::from))
    }

    /// Update the status of a build
    pub async fn update_status(&self, id: i64, status: BuildStatus) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        match status {
            BuildStatus::Running => {
                sqlx::query("UPDATE builds SET status = ?, started_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(&now)
                    .bind(id)
                    .execute(self.db.pool())
                    .await?;
            }
            BuildStatus::Success | BuildStatus::Failed => {
                sqlx::query("UPDATE builds SET status = ?, finished_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(&now)
                    .bind(id)
                    .execute(self.db.pool())
                    .await?;
            }
            _ => {
                sqlx::query("UPDATE builds SET status = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(id)
                    .execute(self.db.pool())
                    .await?;
            }
        }

        Ok(())
    }

    /// Increment retry count and re-queue a build
    pub async fn increment_retry(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE builds SET retry_count = retry_count + 1, status = ? WHERE id = ?")
            .bind(BuildStatus::Queued.as_str())
            .bind(id)
            .execute(self.db.pool())
            .await?;

        Ok(())
    }

    /// Get a specific build by ID
    pub async fn get_build(&self, id: i64) -> Result<Build> {
        let build_row = sqlx::query_as::<_, crate::database::BuildRow>(
            r#"
            SELECT
                b.id, b.commit_hash, b.branch, b.status,
                b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at,
                r.name as repo_name,
                u.username as username
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            JOIN users u ON r.user_id = u.id
            WHERE b.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?
        .ok_or_else(|| DatabaseError::NotFound(format!("Build {} not found", id)))?;

        Ok(Build::from(build_row))
    }

    /// List all builds (for web UI)
    pub async fn list_all_builds(&self) -> Result<Vec<Build>> {
        let build_rows = sqlx::query_as::<_, crate::database::BuildRow>(
            r#"
            SELECT
                b.id, b.commit_hash, b.branch, b.status,
                b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at,
                r.name as repo_name,
                u.username as username
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            JOIN users u ON r.user_id = u.id
            ORDER BY b.created_at DESC
            LIMIT 100
            "#,
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(build_rows.into_iter().map(Build::from).collect())
    }

    /// Get the latest build for a specific repository
    pub async fn get_latest_build_for_repo(&self, repo_name: &str) -> Result<Option<Build>> {
        let build_row = sqlx::query_as::<_, crate::database::BuildRow>(
            r#"
            SELECT
                b.id, b.commit_hash, b.branch, b.status,
                b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at,
                r.name as repo_name,
                u.username as username
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            JOIN users u ON r.user_id = u.id
            WHERE r.name = ?
            ORDER BY b.created_at DESC
            LIMIT 1
            "#,
        )
        .bind(repo_name)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(build_row.map(Build::from))
    }

    // ========================================================================
    // Build Logs
    // ========================================================================

    /// Get build logs for a specific build
    pub async fn get_build_logs(&self, id: i64) -> Result<Option<String>> {
        let log_lines = sqlx::query_scalar::<_, String>(
            "SELECT log_line FROM build_logs WHERE build_id = ? ORDER BY id"
        )
        .bind(id)
        .fetch_all(self.db.pool())
        .await?;

        if log_lines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(log_lines.join("")))
        }
    }

    /// Append a log line to a build (for streaming logs)
    #[allow(dead_code)]
    pub async fn append_log(&self, id: i64, log: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO build_logs (build_id, log_line) VALUES (?, ?)"
        )
        .bind(id)
        .bind(log)
        .execute(self.db.pool())
        .await?;

        Ok(())
    }

    /// Set the complete logs for a build (replaces existing logs)
    pub async fn set_logs(&self, id: i64, logs: &str) -> Result<()> {
        // Delete existing logs
        sqlx::query("DELETE FROM build_logs WHERE build_id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;

        // Insert new logs
        if !logs.is_empty() {
            sqlx::query(
                "INSERT INTO build_logs (build_id, log_line) VALUES (?, ?)"
            )
            .bind(id)
            .bind(logs)
            .execute(self.db.pool())
            .await?;
        }

        Ok(())
    }

    // ========================================================================
    // Build Summaries
    // ========================================================================

    /// Store structured build summary
    pub async fn set_build_summary(&self, id: i64, summary: &BuildSummary) -> Result<()> {
        let summary_json = serde_json::to_string(summary)
            .map_err(|e| DatabaseError::InvalidData(format!("Failed to serialize build summary: {}", e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO build_summaries (build_id, summary_json) VALUES (?, ?)"
        )
        .bind(id)
        .bind(&summary_json)
        .execute(self.db.pool())
        .await?;

        Ok(())
    }

    /// Get structured build summary
    pub async fn get_build_summary(&self, id: i64) -> Result<Option<BuildSummary>> {
        let summary_json = sqlx::query_scalar::<_, String>(
            "SELECT summary_json FROM build_summaries WHERE build_id = ?"
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;

        if let Some(json) = summary_json {
            let summary: BuildSummary = serde_json::from_str(&json)
                .map_err(|e| DatabaseError::InvalidData(format!("Failed to deserialize build summary: {}", e)))?;
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }

    /// Check if build has structured summary
    pub async fn has_build_summary(&self, id: i64) -> Result<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM build_summaries WHERE build_id = ?"
        )
        .bind(id)
        .fetch_one(self.db.pool())
        .await?;

        Ok(count > 0)
    }
}
