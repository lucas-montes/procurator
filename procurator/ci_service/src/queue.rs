//! Build Queue Management
//!
//! Manages the lifecycle of CI builds using SQLite as the backing store.
//! Provides operations for:
//! - Enqueuing new builds from Git hooks
//! - Polling pending builds for the worker
//! - Updating build status (Queued → Running → Success/Failed)
//! - Storing build logs and metadata
//! - Retry logic with configurable maximum attempts
//!
//! The queue is thread-safe and can be shared across multiple tasks via Arc.

use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteConnectOptions, FromRow, SqlitePool};
use tracing::info;

use crate::repo_manager::RepoPath;
use crate::nix_parser::checks::BuildSummary;

#[derive(Debug)]
#[allow(dead_code)]
pub enum QueueError {
    Database(sqlx::Error),
    InvalidStatus(String),
    NotFound(i64),
    ConnectionFailed(String),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::Database(err) => write!(f, "Database error: {}", err),
            QueueError::InvalidStatus(status) => write!(f, "Invalid build status: {}", status),
            QueueError::NotFound(id) => write!(f, "Build not found: {}", id),
            QueueError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
        }
    }
}

impl std::error::Error for QueueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            QueueError::Database(err) => Some(err),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for QueueError {
    fn from(err: sqlx::Error) -> Self {
        QueueError::Database(err)
    }
}

type Result<T> = std::result::Result<T, QueueError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatus {
    Queued,
    Running,
    Success,
    Failed,
}

impl BuildStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildStatus::Queued => "queued",
            BuildStatus::Running => "running",
            BuildStatus::Success => "success",
            BuildStatus::Failed => "failed",
        }
    }
}

impl FromStr for BuildStatus {
    type Err = QueueError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "queued" => Ok(BuildStatus::Queued),
            "running" => Ok(BuildStatus::Running),
            "success" => Ok(BuildStatus::Success),
            "failed" => Ok(BuildStatus::Failed),
            _ => Err(QueueError::InvalidStatus(s.to_string())),
        }
    }
}

impl std::fmt::Display for BuildStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub struct Build {
    pub id: i64,
    pub repo_id: i64,
    pub repo_name: String,
    #[sqlx(rename = "repo_path")]
    repo_path_str: String,
    pub commit_hash: String,
    pub branch: String,
    #[sqlx(try_from = "String")]
    pub status: BuildStatus,
    pub retry_count: i64,
    pub max_retries: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl Build {
    pub fn repo_path(&self, base_path: &str) -> Result<RepoPath> {
        let repo_path_buf = PathBuf::from(&self.repo_path_str);

        let repo_name = repo_path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| QueueError::InvalidStatus("Invalid repo path format".to_string()))?;

        let username = repo_path_buf
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .ok_or_else(|| QueueError::InvalidStatus("Invalid repo path format".to_string()))?;

        Ok(RepoPath::new(base_path, username, repo_name))
    }
}

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub struct Repo {
    pub id: i64,
    pub name: String,
    path: String,
    pub description: Option<String>,
    pub created_at: String,
}

impl Repo {
    pub fn path(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

// Implement conversion from String for sqlx
impl TryFrom<String> for BuildStatus {
    type Error = QueueError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse()
    }
}

pub struct BuildQueue {
    pool: SqlitePool,
}

impl BuildQueue {
    pub async fn new(database_url: &str) -> Result<Self> {
        let database_config = SqliteConnectOptions::from_str(database_url)
            .expect("Cannot connect to database")
            .create_if_missing(true);

        let pool = SqlitePool::connect_lazy_with(database_config);

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS repos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                path TEXT NOT NULL UNIQUE,
                description TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS builds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id INTEGER NOT NULL,
                commit_hash TEXT NOT NULL,
                branch TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                finished_at TEXT,
                FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS build_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                build_id INTEGER NOT NULL,
                log_line TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (build_id) REFERENCES builds(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&pool)
        .await?;

        // Table for structured build summaries
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS build_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                build_id INTEGER NOT NULL UNIQUE,
                summary_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (build_id) REFERENCES builds(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&pool)
        .await?;

        // Create indexes for performance
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_repo_id ON builds(repo_id)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_status ON builds(status)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_build_logs_build_id ON build_logs(build_id)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_build_summaries_build_id ON build_summaries(build_id)")
            .execute(&pool)
            .await?;

        info!("Database initialized at {}", database_url);
        Ok(Self { pool })
    }

    /// Get or create a repo by path, returning its ID
    async fn get_or_create_repo(&self, repo_path: &str) -> Result<i64> {
        // Extract repo name from path (remove .git extension if present)
        let path_buf = std::path::PathBuf::from(repo_path);
        let repo_name = path_buf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        // Try to get existing repo
        if let Some(repo) = sqlx::query_as::<_, Repo>(
            "SELECT id, name, path, description, created_at FROM repos WHERE path = ?"
        )
        .bind(repo_path)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(repo.id);
        }

        // Create new repo
        let result = sqlx::query(
            "INSERT INTO repos (name, path) VALUES (?, ?)"
        )
        .bind(repo_name)
        .bind(repo_path)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn enqueue(&self, repo_path: PathBuf, commit: &str, branch: &str) -> Result<i64> {
        let repo_path_str = repo_path.to_string_lossy().to_string();
        let repo_id = self.get_or_create_repo(&repo_path_str).await?;

        let result = sqlx::query(
            "INSERT INTO builds (repo_id, commit_hash, branch) VALUES (?, ?, ?)"
        )
        .bind(repo_id)
        .bind(commit)
        .bind(branch)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_pending(&self) -> Result<Option<Build>> {
        let build = sqlx::query_as::<_, Build>(
            r#"
            SELECT
                b.id, b.repo_id, r.name as repo_name, r.path as repo_path,
                b.commit_hash, b.branch, b.status, b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            WHERE b.status = ?
            ORDER BY b.created_at
            LIMIT 1
            "#,
        )
        .bind(BuildStatus::Queued.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(build)
    }

    pub async fn update_status(&self, id: i64, status: BuildStatus) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        match status {
            BuildStatus::Running => {
                sqlx::query("UPDATE builds SET status = ?, started_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            BuildStatus::Success | BuildStatus::Failed => {
                sqlx::query("UPDATE builds SET status = ?, finished_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            _ => {
                sqlx::query("UPDATE builds SET status = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn increment_retry(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE builds SET retry_count = retry_count + 1, status = ? WHERE id = ?")
            .bind(BuildStatus::Queued.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn can_retry(&self, build: &Build) -> bool {
        build.retry_count < build.max_retries
    }

    // Web UI methods
    pub async fn list_all_builds(&self) -> Result<Vec<Build>> {
        let builds = sqlx::query_as::<_, Build>(
            r#"
            SELECT
                b.id, b.repo_id, r.name as repo_name, r.path as repo_path,
                b.commit_hash, b.branch, b.status, b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            ORDER BY b.created_at DESC
            LIMIT 100
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(builds)
    }

    pub async fn get_build(&self, id: i64) -> Result<Build> {
        let build = sqlx::query_as::<_, Build>(
            r#"
            SELECT
                b.id, b.repo_id, r.name as repo_name, r.path as repo_path,
                b.commit_hash, b.branch, b.status, b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            WHERE b.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(QueueError::NotFound(id))?;

        Ok(build)
    }

    pub async fn list_repos(&self) -> Result<Vec<(String, String, i64)>> {
        let repos = sqlx::query_as::<_, (String, String, i64)>(
            r#"
            SELECT
                r.name,
                r.path,
                COUNT(b.id) as builds_count
            FROM repos r
            LEFT JOIN builds b ON r.id = b.repo_id
            GROUP BY r.id, r.name, r.path
            ORDER BY r.created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(repos)
    }

    pub async fn get_latest_build_for_repo(&self, repo_name: &str) -> Result<Option<Build>> {
        let build = sqlx::query_as::<_, Build>(
            r#"
            SELECT
                b.id, b.repo_id, r.name as repo_name, r.path as repo_path,
                b.commit_hash, b.branch, b.status, b.retry_count, b.max_retries,
                b.created_at, b.started_at, b.finished_at
            FROM builds b
            JOIN repos r ON b.repo_id = r.id
            WHERE r.name = ?
            ORDER BY b.created_at DESC
            LIMIT 1
            "#,
        )
        .bind(repo_name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(build)
    }

    pub async fn get_build_logs(&self, id: i64) -> Result<Option<String>> {
        let log_lines = sqlx::query_scalar::<_, String>(
            "SELECT log_line FROM build_logs WHERE build_id = ? ORDER BY id"
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        if log_lines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(log_lines.join("")))
        }
    }

    #[allow(dead_code)]
    pub async fn append_log(&self, id: i64, log: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO build_logs (build_id, log_line) VALUES (?, ?)"
        )
        .bind(id)
        .bind(log)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn set_logs(&self, id: i64, logs: &str) -> Result<()> {
        // Delete existing logs
        sqlx::query("DELETE FROM build_logs WHERE build_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        // Insert new logs
        if !logs.is_empty() {
            sqlx::query(
                "INSERT INTO build_logs (build_id, log_line) VALUES (?, ?)"
            )
            .bind(id)
            .bind(logs)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Store structured build summary
    pub async fn set_build_summary(&self, id: i64, summary: &BuildSummary) -> Result<()> {
        let summary_json = serde_json::to_string(summary)
            .map_err(|e| QueueError::InvalidStatus(format!("Failed to serialize build summary: {}", e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO build_summaries (build_id, summary_json) VALUES (?, ?)"
        )
        .bind(id)
        .bind(&summary_json)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get structured build summary
    pub async fn get_build_summary(&self, id: i64) -> Result<Option<BuildSummary>> {
        let summary_json = sqlx::query_scalar::<_, String>(
            "SELECT summary_json FROM build_summaries WHERE build_id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(json) = summary_json {
            let summary: BuildSummary = serde_json::from_str(&json)
                .map_err(|e| QueueError::InvalidStatus(format!("Failed to deserialize build summary: {}", e)))?;
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
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_path_components() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        assert_eq!(path.repo_name(), "myrepo");
    }

    #[test]
    fn test_repo_path_bare_repo_path() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let bare_path = path.bare_repo_path();
        assert_eq!(bare_path, PathBuf::from("/base/path/testuser/myrepo.git"));
    }

    #[test]
    fn test_repo_path_to_ssh_url() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let ssh_url = path.to_ssh_url("example.com");
        assert_eq!(ssh_url, "git@example.com:testuser/myrepo.git");
    }

    #[test]
    fn test_repo_path_to_nix_url() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let nix_url = path.to_nix_url();
        assert_eq!(nix_url, "git+file:///base/path/testuser/myrepo.git");
    }

    #[test]
    fn test_repo_path_to_nix_url_with_rev() {
        let path = RepoPath::new("/base/path", "testuser", "myrepo");
        let nix_url = path.to_nix_url_with_rev("abc123");
        assert_eq!(nix_url, "git+file:///base/path/testuser/myrepo.git?rev=abc123");
    }

    #[test]
    fn test_build_repo_path_parsing() {
        let build = Build {
            id: 1,
            repo_id: 1,
            repo_name: "test-repo".to_string(),
            repo_path_str: "/base/testuser/test-repo.git".to_string(),
            commit_hash: "abc123".to_string(),
            branch: "main".to_string(),
            status: BuildStatus::Queued,
            retry_count: 0,
            max_retries: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            started_at: None,
            finished_at: None,
        };

        let repo_path = build.repo_path("/base").unwrap();
        assert_eq!(repo_path.repo_name(), "test-repo");
    }

    #[test]
    fn test_build_repo_path_invalid_format() {
        let build = Build {
            id: 1,
            repo_id: 1,
            repo_name: "test-repo".to_string(),
            repo_path_str: "/invalid".to_string(), // No parent directory
            commit_hash: "abc123".to_string(),
            branch: "main".to_string(),
            status: BuildStatus::Queued,
            retry_count: 0,
            max_retries: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            started_at: None,
            finished_at: None,
        };

        let result = build.repo_path("/base");
        assert!(result.is_err());
    }

    #[test]
    fn test_repo_path_method() {
        let repo = Repo {
            id: 1,
            name: "test-repo".to_string(),
            path: "/repos/user/test-repo.git".to_string(),
            description: Some("Test description".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert_eq!(repo.path(), PathBuf::from("/repos/user/test-repo.git"));
    }

    #[test]
    fn test_build_status_all_variants() {
        let variants = vec![
            BuildStatus::Queued,
            BuildStatus::Running,
            BuildStatus::Success,
            BuildStatus::Failed,
        ];

        for variant in variants {
            // Test round-trip through string
            let as_str = variant.as_str();
            let parsed: BuildStatus = as_str.parse().unwrap();
            assert_eq!(variant, parsed);
        }
    }

    #[test]
    fn test_queue_error_source() {
        use std::error::Error;

        let sqlx_err = sqlx::Error::RowNotFound;
        let queue_err = QueueError::Database(sqlx_err);

        assert!(queue_err.source().is_some());

        let status_err = QueueError::InvalidStatus("test".to_string());
        assert!(status_err.source().is_none());
    }

    #[test]
    fn test_multiple_repo_paths() {
        let paths = vec![
            RepoPath::new("/base", "user1", "repo1"),
            RepoPath::new("/base", "user2", "repo2"),
            RepoPath::new("/other", "user3", "repo3"),
        ];

        assert_eq!(paths[0].repo_name(), "repo1");
        assert_eq!(paths[1].repo_name(), "repo2");
        assert_eq!(paths[2].repo_name(), "repo3");

        assert_eq!(paths[0].bare_repo_path(), PathBuf::from("/base/user1/repo1.git"));
        assert_eq!(paths[1].bare_repo_path(), PathBuf::from("/base/user2/repo2.git"));
        assert_eq!(paths[2].bare_repo_path(), PathBuf::from("/other/user3/repo3.git"));
    }

    #[test]
    fn test_build_struct_fields() {
        let build = Build {
            id: 42,
            repo_id: 10,
            repo_name: "my-repo".to_string(),
            repo_path_str: "/repos/user/my-repo.git".to_string(),
            commit_hash: "abc123def456".to_string(),
            branch: "feature-branch".to_string(),
            status: BuildStatus::Running,
            retry_count: 2,
            max_retries: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            started_at: Some("2024-01-01T00:01:00Z".to_string()),
            finished_at: None,
        };

        assert_eq!(build.id, 42);
        assert_eq!(build.repo_id, 10);
        assert_eq!(build.repo_name, "my-repo");
        assert_eq!(build.commit_hash, "abc123def456");
        assert_eq!(build.branch, "feature-branch");
        assert_eq!(build.status, BuildStatus::Running);
        assert_eq!(build.retry_count, 2);
        assert_eq!(build.max_retries, 5);
        assert!(build.started_at.is_some());
        assert!(build.finished_at.is_none());
    }

    #[test]
    fn test_repo_struct_fields() {
        let repo = Repo {
            id: 1,
            name: "my-repo".to_string(),
            path: "/repos/user/my-repo.git".to_string(),
            description: Some("A test repository".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert_eq!(repo.id, 1);
        assert_eq!(repo.name, "my-repo");
        assert_eq!(repo.description.as_deref(), Some("A test repository"));
        assert_eq!(repo.created_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_repo_struct_no_description() {
        let repo = Repo {
            id: 1,
            name: "my-repo".to_string(),
            path: "/repos/user/my-repo.git".to_string(),
            description: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert!(repo.description.is_none());
    }

    #[test]
    fn test_build_status_clone_and_copy() {
        let status1 = BuildStatus::Queued;
        let status2 = status1; // Copy
        let status3 = status1.clone(); // Clone

        assert_eq!(status1, status2);
        assert_eq!(status1, status3);
        assert_eq!(status2, status3);
    }

    #[test]
    fn test_build_clone() {
        let build = Build {
            id: 1,
            repo_id: 1,
            repo_name: "test".to_string(),
            repo_path_str: "/path".to_string(),
            commit_hash: "abc".to_string(),
            branch: "main".to_string(),
            status: BuildStatus::Queued,
            retry_count: 0,
            max_retries: 3,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            started_at: None,
            finished_at: None,
        };

        let cloned = build.clone();
        assert_eq!(build.id, cloned.id);
        assert_eq!(build.commit_hash, cloned.commit_hash);
        assert_eq!(build.status, cloned.status);
    }

    #[test]
    fn test_pathbuf_extraction() {
        let path_str = "/repos/user/my-repo.git";
        let path_buf = PathBuf::from(path_str);

        // Test file_stem extraction (removes .git)
        let stem = path_buf.file_stem().and_then(|s| s.to_str());
        assert_eq!(stem, Some("my-repo"));

        // Test parent extraction
        let parent = path_buf.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str());
        assert_eq!(parent, Some("user"));
    }

    #[test]
    fn test_pathbuf_edge_cases() {
        // Path with multiple dots
        let path_buf = PathBuf::from("/repos/user/my.repo.test.git");
        let stem = path_buf.file_stem().and_then(|s| s.to_str());
        assert_eq!(stem, Some("my.repo.test"));

        // Path without extension
        let path_buf = PathBuf::from("/repos/user/myrepo");
        let stem = path_buf.file_stem().and_then(|s| s.to_str());
        assert_eq!(stem, Some("myrepo"));
    }

    #[test]
    fn test_error_variants_debug() {
        let err1 = QueueError::InvalidStatus("test".to_string());
        let debug_str = format!("{:?}", err1);
        assert!(debug_str.contains("InvalidStatus"));

        let err2 = QueueError::NotFound(42);
        let debug_str = format!("{:?}", err2);
        assert!(debug_str.contains("NotFound"));
    }
}
