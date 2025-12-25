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

// std::path::PathBuf was unused here previously

use crate::database::{Database, DatabaseError, BuildSummary};
use crate::builds::{BuildStatus, BuildJob};


pub type Result<T> = std::result::Result<T, DatabaseError>;




#[derive(Clone)]
pub struct JobQueue {
    db: Database,
}

impl JobQueue {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Enqueue a new build job
    pub async fn enqueue(&self, repo_path: &str, commit: &str, branch: &str) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO builds (repo_path, commit_hash, branch, status) VALUES (?, ?, ?, 'queued')"
        )
        .bind(repo_path)
        .bind(commit)
        .bind(branch)
        .execute(&*self.db)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Get the next pending build from the queue
    pub async fn get_pending(&self) -> Result<Option<BuildJob>> {
        // Atomic claim: update a single queued row to running and read it back.

        let now = chrono::Utc::now().to_rfc3339();

                let updated = sqlx::query(
                        r#"UPDATE builds SET status = 'running', started_at = ?
                             WHERE id = (
                                 SELECT id FROM builds WHERE status = 'queued' ORDER BY created_at LIMIT 1
                             )"#,
                )
                .bind(&now)
                .execute(&*self.db)
                .await?;

        if updated.rows_affected() == 0 {
            return Ok(None);
        }

        #[derive(sqlx::FromRow)]
        struct Row {
            id: i64,
            repo_path: String,
            commit_hash: String,
            branch: String,
            status: String,
            retry_count: i64,
            max_retries: i64,
            created_at: String,
            started_at: Option<String>,
            finished_at: Option<String>,
        }

        let job = sqlx::query_as::<_, BuildJob>(
            r#"SELECT id, repo_path, commit_hash, branch, status, retry_count, max_retries, created_at, started_at, finished_at
               FROM builds WHERE status = 'running' ORDER BY started_at LIMIT 1"#
        )
        .fetch_one(&*self.db)
        .await?;

        Ok(Some(job))
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
                    .execute(&*self.db)
                    .await?;
            }
            BuildStatus::Success | BuildStatus::Failed => {
                sqlx::query("UPDATE builds SET status = ?, finished_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(&now)
                    .bind(id)
                    .execute(&*self.db)
                    .await?;
            }
            _ => {
                sqlx::query("UPDATE builds SET status = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(id)
                    .execute(&*self.db)
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
            .execute(&*self.db)
            .await?;

        Ok(())
    }

    /// Get a specific build by ID
    pub async fn get_build(&self, id: i64) -> Result<BuildJob> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i64,
            repo_path: String,
            commit_hash: String,
            branch: String,
            status: String,
            retry_count: i64,
            max_retries: i64,
            created_at: String,
            started_at: Option<String>,
            finished_at: Option<String>,
        }

        let job = sqlx::query_as::<_, BuildJob>(
            r#"
            SELECT
                id, repo_path, commit_hash, branch, status,
                retry_count, max_retries,
                created_at, started_at, finished_at
            FROM builds
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&*self.db)
        .await?
        .ok_or_else(|| DatabaseError::NotFound(format!("Build {} not found", id)))?;

        Ok(job)
    }

    /// List all builds (for web UI)
    pub async fn list_all_builds(&self) -> Result<Vec<BuildJob>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i64,
            repo_path: String,
            commit_hash: String,
            branch: String,
            status: String,
            retry_count: i64,
            max_retries: i64,
            created_at: String,
            started_at: Option<String>,
            finished_at: Option<String>,
        }

        let jobs = sqlx::query_as::<_, BuildJob>(
            r#"
            SELECT
                id, repo_path, commit_hash, branch, status,
                retry_count, max_retries,
                created_at, started_at, finished_at
            FROM builds
            ORDER BY created_at DESC
            LIMIT 100
            "#,
        )
        .fetch_all(&*self.db)
        .await?;

        Ok(jobs)
    }

    /// Get the latest build for a specific repository
    /// Get the latest build for a repository prefix (matches repo_path)
    pub async fn get_latest_build_for_repo(&self, repo_path_prefix: &str) -> Result<Option<BuildJob>> {

        let job = sqlx::query_as::<_, BuildJob>(
            r#"
            SELECT
                id, repo_path, commit_hash, branch, status,
                retry_count, max_retries,
                created_at, started_at, finished_at
            FROM builds
            WHERE repo_path LIKE ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(format!("{}%", repo_path_prefix))
        .fetch_optional(&*self.db)
        .await?;

        Ok(job)
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
        .fetch_all(&*self.db)
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
        .execute(&*self.db)
        .await?;

        Ok(())
    }

    /// Set the complete logs for a build (replaces existing logs)
    pub async fn set_logs(&self, id: i64, logs: &str) -> Result<()> {
        // Delete existing logs
        sqlx::query("DELETE FROM build_logs WHERE build_id = ?")
            .bind(id)
            .execute(&*self.db)
            .await?;

        // Insert new logs
        if !logs.is_empty() {
            sqlx::query(
                "INSERT INTO build_logs (build_id, log_line) VALUES (?, ?)"
            )
            .bind(id)
            .bind(logs)
            .execute(&*self.db)
            .await?;
        }

        Ok(())
    }


    /// Store structured build summary
    pub async fn set_build_summary(&self, id: i64, summary: &BuildSummary) -> Result<()> {
        let summary_json = serde_json::to_string(summary)
            .map_err(|e| DatabaseError::InvalidData(format!("Failed to serialize build summary: {}", e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO build_summaries (build_id, summary_json) VALUES (?, ?)"
        )
        .bind(id)
        .bind(&summary_json)
        .execute(&*self.db)
        .await?;

        Ok(())
    }

    /// Get structured build summary
    pub async fn get_build_summary(&self, id: i64) -> Result<Option<BuildSummary>> {
        let summary_json = sqlx::query_scalar::<_, String>(
            "SELECT summary_json FROM build_summaries WHERE build_id = ?"
        )
        .bind(id)
        .fetch_optional(&*self.db)
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
        .fetch_one(&*self.db)
        .await?;

        Ok(count > 0)
    }
}
