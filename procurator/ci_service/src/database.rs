//! Database Infrastructure Layer
//!
//! Handles database connection, schema initialization, and provides
//! data transfer objects (DTOs) for mapping between database rows and domain models.
//!
//! This layer is responsible ONLY for database concerns - no business logic.

use std::{ops::Deref, str::FromStr};

use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use tracing::info;

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

// DTO for reading builds from simplified tables
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BuildRow {
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

impl BuildRow {
    pub fn id(&self) -> i64 { self.id }
    pub fn repo_path(&self) -> &str { &self.repo_path }
    pub fn commit_hash(&self) -> &str { &self.commit_hash }
    pub fn branch(&self) -> &str { &self.branch }
    pub fn status(&self) -> &str { &self.status }
    pub fn retry_count(&self) -> i64 { self.retry_count }
    pub fn max_retries(&self) -> i64 { self.max_retries }
    pub fn created_at(&self) -> &str { &self.created_at }
    pub fn started_at(&self) -> Option<&str> { self.started_at.as_deref() }
    pub fn finished_at(&self) -> Option<&str> { self.finished_at.as_deref() }
}

// Structured summary placeholder - stores arbitrary JSON produced by the runner
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BuildSummary {
    pub total_duration_seconds: Option<i64>,
    pub steps: Vec<String>,
    pub packages_checked: Vec<String>,
    pub checks_run: Vec<String>,
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Deref for Database {
    type Target = SqlitePool;
    fn deref(&self) -> &Self::Target {
        &self.pool
    }
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
        // Simplified single table for builds. Status will be used to track lifecycle
        // (queued, running, success, failed, canceled). This keeps schema simple
        // while allowing efficient queries and straightforward migrations.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS builds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_path TEXT NOT NULL,
                commit_hash TEXT NOT NULL,
                branch TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                finished_at TEXT,
                last_heartbeat TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Build logs table (referencing builds.build_id)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS build_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                build_id INTEGER NOT NULL,
                log_line TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
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
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;


        // Create indexes for performance
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_status_created ON builds(status, created_at)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_builds_repo_path ON builds(repo_path)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_build_logs_build_id ON build_logs(build_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_build_summaries_build_id ON build_summaries(build_id)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
