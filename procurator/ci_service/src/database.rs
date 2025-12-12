//! Database Infrastructure Layer
//!
//! Handles database connection, schema initialization, and provides
//! data transfer objects (DTOs) for mapping between database rows and domain models.
//!
//! This layer is responsible ONLY for database concerns - no business logic.

use std::str::FromStr;

use sqlx::{sqlite::SqliteConnectOptions, FromRow, SqlitePool};
use tracing::info;

use crate::domain::{Build, BuildId, BuildStatus, CommitInfo, RepositoryInfo, RetryInfo, Timestamps};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug)]
pub enum DatabaseError {
    Connection(sqlx::Error),
    Query(sqlx::Error),
    InvalidData(String),
    NotFound(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::Connection(err) => write!(f, "Database connection error: {}", err),
            DatabaseError::Query(err) => write!(f, "Database query error: {}", err),
            DatabaseError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
            DatabaseError::NotFound(msg) => write!(f, "Not found: {}", msg),
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DatabaseError::Connection(err) | DatabaseError::Query(err) => Some(err),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for DatabaseError {
    fn from(err: sqlx::Error) -> Self {
        DatabaseError::Query(err)
    }
}

pub type Result<T> = std::result::Result<T, DatabaseError>;

// ============================================================================
// Data Transfer Objects (DTOs)
// ============================================================================

/// Database row with JOINed repository and user information
/// Maps directly to SQL query results
#[derive(Debug, Clone, FromRow)]
pub struct BuildRow {
    pub id: i64,
    pub commit_hash: String,
    pub branch: String,
    #[sqlx(try_from = "String")]
    pub status: BuildStatus,
    pub retry_count: i64,
    pub max_retries: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    // JOINed from repos table
    pub repo_name: String,
    // JOINed from users table
    pub username: String,
}

// ============================================================================
// Mapping: Database → Domain
// ============================================================================

impl From<BuildRow> for Build {
    fn from(row: BuildRow) -> Self {
        Build::new(
            BuildId(row.id),
            RepositoryInfo::new(row.username, row.repo_name),
            CommitInfo::new(row.commit_hash, row.branch),
            row.status,
            RetryInfo::new(row.retry_count, row.max_retries),
            Timestamps::new(row.created_at, row.started_at, row.finished_at),
        )
    }
}

// ============================================================================
// Database Core
// ============================================================================

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        let database_config = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| DatabaseError::Connection(e))?
            .create_if_missing(true);

        let pool = SqlitePool::connect_lazy_with(database_config);

        let db = Self { pool };
        db.initialize_tables().await?;

        info!("Database initialized at {}", database_url);
        Ok(db)
    }

    async fn initialize_tables(&self) -> Result<()> {
        // Users table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                email TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Repos table (now with user_id foreign key)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS repos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                description TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                UNIQUE(user_id, name)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Builds table
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
        .execute(&self.pool)
        .await?;

        // Build logs table
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
        .execute(&self.pool)
        .await?;

        // Build summaries table
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
        .execute(&self.pool)
        .await?;

        // Create indexes for performance
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_repos_user_id ON repos(user_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_repo_id ON builds(repo_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_status ON builds(status)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_build_logs_build_id ON build_logs(build_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_build_summaries_build_id ON build_summaries(build_id)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
