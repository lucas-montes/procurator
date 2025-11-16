use std::str::FromStr;

use sqlx::{FromRow, SqlitePool, sqlite::SqliteConnectOptions};
use tracing::info;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, FromRow)]
pub struct Build {
    pub id: i64,
    pub repo: String,
    pub commit_hash: String,
    pub branch: String,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
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
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                finished_at TEXT
            )
            "#,
        )
        .execute(&pool)
        .await?;

        info!("Database initialized at {}", database_url);
        Ok(Self { pool })
    }

    pub async fn enqueue(&self, repo: &str, commit: &str, branch: &str) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO builds (repo, commit_hash, branch) VALUES (?, ?, ?)"
        )
        .bind(repo)
        .bind(commit)
        .bind(branch)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn get_pending(&self) -> Result<Option<Build>> {
        let build = sqlx::query_as::<_, Build>(
            "SELECT * FROM builds WHERE status = 'queued' ORDER BY created_at LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(build)
    }

    pub async fn update_status(&self, id: i64, status: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        match status {
            "running" => {
                sqlx::query("UPDATE builds SET status = ?, started_at = ? WHERE id = ?")
                    .bind(status)
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            "success" | "failed" => {
                sqlx::query("UPDATE builds SET status = ?, finished_at = ? WHERE id = ?")
                    .bind(status)
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            _ => {
                sqlx::query("UPDATE builds SET status = ? WHERE id = ?")
                    .bind(status)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        Ok(())
    }
}
