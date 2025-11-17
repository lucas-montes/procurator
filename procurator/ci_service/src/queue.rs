use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteConnectOptions, FromRow, SqlitePool};
use tracing::info;

#[derive(Debug)]
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
pub struct Build {
    pub id: i64,
    pub repo: String,
    pub commit_hash: String,
    pub branch: String,
    #[sqlx(try_from = "String")]
    pub status: BuildStatus,
    pub retry_count: i64,
    pub max_retries: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub logs: Option<String>,
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

        // Create table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS builds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                branch TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                finished_at TEXT,
                logs TEXT
            )
            "#,
        )
        .execute(&pool)
        .await?;

        info!("Database initialized at {}", database_url);
        Ok(Self { pool })
    }

    pub async fn enqueue(&self, repo: &str, commit: &str, branch: &str) -> Result<i64> {
        let result = sqlx::query("INSERT INTO builds (repo, commit_hash, branch) VALUES (?, ?, ?)")
            .bind(repo)
            .bind(commit)
            .bind(branch)
            .execute(&self.pool)
            .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_pending(&self) -> Result<Option<Build>> {
        let build = sqlx::query_as::<_, Build>(
            "SELECT * FROM builds WHERE status = ? ORDER BY created_at LIMIT 1",
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
            "SELECT * FROM builds ORDER BY created_at DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(builds)
    }

    pub async fn get_build(&self, id: i64) -> Result<Build> {
        let build = sqlx::query_as::<_, Build>("SELECT * FROM builds WHERE id = ?")
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
                repo as name,
                repo as path,
                COUNT(*) as builds_count
            FROM builds
            GROUP BY repo
            ORDER BY MAX(created_at) DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(repos)
    }

    pub async fn get_latest_build_for_repo(&self, repo: &str) -> Result<Option<Build>> {
        let build = sqlx::query_as::<_, Build>(
            "SELECT * FROM builds WHERE repo = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(repo)
        .fetch_optional(&self.pool)
        .await?;

        Ok(build)
    }

    pub async fn get_build_logs(&self, id: i64) -> Result<Option<String>> {
        let logs = sqlx::query_scalar::<_, Option<String>>(
            "SELECT logs FROM builds WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .flatten();

        Ok(logs)
    }

    pub async fn append_log(&self, id: i64, log: &str) -> Result<()> {
        sqlx::query(
            "UPDATE builds SET logs = COALESCE(logs || ?, ?) WHERE id = ?"
        )
        .bind(format!("{}\n", log))
        .bind(format!("{}\n", log))
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn set_logs(&self, id: i64, logs: &str) -> Result<()> {
        sqlx::query("UPDATE builds SET logs = ? WHERE id = ?")
            .bind(logs)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
