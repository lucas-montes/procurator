//! `SQLite` run store. Why: one file, transactional, no extra service to run.

use std::future::Future;
use std::str::FromStr as _;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row as _, SqlitePool};

use crate::application::ports::{RunQuery, RunStore, StoreError};
use crate::domain::run::{
    CommitHash, GitRef, NewRun, Pusher, RepoName, Run, RunId, RunStatus, RunTimes,
};

/// Tables and indexes, created on open.
/// Why: the table is `runs`, not the scaffold's `builds`, so an old database
/// opens cleanly; the old tables are simply ignored.
const SCHEMA: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        repo TEXT NOT NULL,
        commit_hash TEXT NOT NULL,
        git_ref TEXT NOT NULL,
        pusher TEXT,
        status TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        started_at INTEGER,
        finished_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS runs_status ON runs (status, id)",
    "CREATE INDEX IF NOT EXISTS runs_repo ON runs (repo, id)",
];

/// Columns every query reads back, in [`run_from_row`]'s order.
const COLUMNS: &str =
    "id, repo, commit_hash, git_ref, pusher, status, created_at, started_at, finished_at";

/// Runs stored in one `SQLite` file.
#[derive(Debug)]
pub struct SqliteRunStore {
    pool: SqlitePool,
}

impl SqliteRunStore {
    /// Open (creating if needed) the store at a `sqlite://` URL.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the URL is invalid, the file cannot be opened, or
    /// the schema cannot be created.
    pub fn open(url: &str) -> impl Future<Output = Result<Self, StoreError>> + Send {
        let options = SqliteConnectOptions::from_str(url).map(|options| {
            options
                .create_if_missing(true)
                // Why: WAL lets the API read while a worker writes.
                .journal_mode(SqliteJournalMode::Wal)
        });
        async move { Self::connect(options.map_err(StoreError::new)?, 4).await }
    }

    /// A private in-memory store. Why: tests of this adapter.
    #[cfg(test)]
    fn in_memory() -> impl Future<Output = Result<Self, StoreError>> + Send {
        async {
            let options =
                SqliteConnectOptions::from_str("sqlite::memory:").map_err(StoreError::new)?;
            // Why: each in-memory connection is its own database, so use one.
            Self::connect(options, 1).await
        }
    }

    fn connect(
        options: SqliteConnectOptions,
        connections: u32,
    ) -> impl Future<Output = Result<Self, StoreError>> + Send {
        async move {
            let pool = SqlitePoolOptions::new()
                .max_connections(connections)
                .connect_with(options)
                .await
                .map_err(StoreError::new)?;
            for statement in SCHEMA {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .map_err(StoreError::new)?;
            }
            Ok(Self { pool })
        }
    }
}

/// Rebuild a run from a row selected with [`COLUMNS`].
/// Why: rows are re-validated, so a corrupt row is an error, not a bad run.
fn run_from_row(row: &SqliteRow) -> Result<Run, StoreError> {
    let text = |column: &str| row.try_get::<String, _>(column).map_err(StoreError::new);
    let request = NewRun::new(
        RepoName::parse(text("repo")?).map_err(StoreError::new)?,
        CommitHash::parse(text("commit_hash")?).map_err(StoreError::new)?,
        GitRef::parse(text("git_ref")?).map_err(StoreError::new)?,
        row.try_get::<Option<String>, _>("pusher")
            .map_err(StoreError::new)?
            .map(Pusher::parse)
            .transpose()
            .map_err(StoreError::new)?,
    );
    let status: &str = row.try_get("status").map_err(StoreError::new)?;
    let times = RunTimes::new(
        row.try_get("created_at").map_err(StoreError::new)?,
        row.try_get("started_at").map_err(StoreError::new)?,
        row.try_get("finished_at").map_err(StoreError::new)?,
    );
    Ok(Run::from_parts(
        RunId::new(row.try_get("id").map_err(StoreError::new)?),
        request,
        RunStatus::parse(status).map_err(StoreError::new)?,
        times,
    ))
}

impl RunStore for SqliteRunStore {
    fn insert(
        &self,
        run: NewRun,
        now: i64,
    ) -> impl Future<Output = Result<RunId, StoreError>> + Send {
        async move {
            let result = sqlx::query(
                "INSERT INTO runs (repo, commit_hash, git_ref, pusher, status, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(run.repo().as_str())
            .bind(run.commit().as_str())
            .bind(run.git_ref().as_str())
            .bind(run.pusher().map(Pusher::as_str))
            .bind(RunStatus::Queued.as_str())
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(StoreError::new)?;
            Ok(RunId::new(result.last_insert_rowid()))
        }
    }

    fn claim_next(&self, now: i64) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        async move {
            // Why: one statement picks and claims, so two workers can never
            // claim the same run and the row returned is the row updated.
            let sql = format!(
                "UPDATE runs SET status = 'running', started_at = ?
                 WHERE id = (SELECT id FROM runs WHERE status = 'queued' ORDER BY id LIMIT 1)
                 RETURNING {COLUMNS}"
            );
            let row = sqlx::query(&sql)
                .bind(now)
                .fetch_optional(&self.pool)
                .await
                .map_err(StoreError::new)?;
            row.as_ref().map(run_from_row).transpose()
        }
    }

    fn requeue_running(&self) -> impl Future<Output = Result<u64, StoreError>> + Send {
        async move {
            let result = sqlx::query(
                "UPDATE runs SET status = 'queued', started_at = NULL WHERE status = 'running'",
            )
            .execute(&self.pool)
            .await
            .map_err(StoreError::new)?;
            Ok(result.rows_affected())
        }
    }

    fn finish(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        async move {
            if !status.is_final() {
                return Ok(false);
            }
            let result = sqlx::query(
                "UPDATE runs SET status = ?, finished_at = ? WHERE id = ? AND status = 'running'",
            )
            .bind(status.as_str())
            .bind(now)
            .bind(id.get())
            .execute(&self.pool)
            .await
            .map_err(StoreError::new)?;
            Ok(result.rows_affected() == 1)
        }
    }

    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>, StoreError>> + Send {
        async move {
            let sql = format!("SELECT {COLUMNS} FROM runs WHERE id = ?");
            let row = sqlx::query(&sql)
                .bind(id.get())
                .fetch_optional(&self.pool)
                .await
                .map_err(StoreError::new)?;
            row.as_ref().map(run_from_row).transpose()
        }
    }

    fn runs(
        &self,
        query: RunQuery<'_>,
    ) -> impl Future<Output = Result<Vec<Run>, StoreError>> + Send {
        async move {
            let sql = format!(
                "SELECT {COLUMNS} FROM runs
                 WHERE (?1 IS NULL OR repo = ?1) AND (?2 IS NULL OR id < ?2)
                 ORDER BY id DESC LIMIT ?3"
            );
            let rows = sqlx::query(&sql)
                .bind(query.repo().map(RepoName::as_str))
                .bind(query.before().map(RunId::get))
                .bind(query.limit())
                .fetch_all(&self.pool)
                .await
                .map_err(StoreError::new)?;
            rows.iter().map(run_from_row).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteRunStore;
    use crate::adapters::store_contract;
    use crate::application::ports::RunStore as _;
    use crate::domain::run::RunStatus;

    #[tokio::test]
    async fn satisfies_the_store_contract() {
        store_contract::check(&SqliteRunStore::in_memory().await.unwrap()).await;
    }

    #[tokio::test]
    async fn runs_survive_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("ci.db").display());
        let store = SqliteRunStore::open(&url).await.unwrap();
        store_contract::check(&store).await;
        let before = store.runs(crate::application::ports::RunQuery::new(None, None, 100));
        let before = before.await.unwrap();
        drop(store);

        let reopened = SqliteRunStore::open(&url).await.unwrap();
        let after = reopened
            .runs(crate::application::ports::RunQuery::new(None, None, 100))
            .await
            .unwrap();
        assert_eq!(before, after);
        assert!(after.iter().all(|run| run.status() != RunStatus::Running));
    }
}
