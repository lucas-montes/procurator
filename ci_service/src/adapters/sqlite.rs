//! `SQLite` run store. Why: one file, transactional, no extra service to run.

use std::future::Future;
use std::str::FromStr as _;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row as _, SqlitePool};

use crate::application::ports::{NextAttempt, RunQuery, RunStore, StoreError};
use crate::domain::check::{Check, CheckId, CheckStatus, Derivation, NewCheck};
use crate::domain::event::{Delivery, EventPayload, Target};
use crate::domain::log::{LogLine, LogOrigin, NewLogLine};
use crate::domain::run::{
    CommitHash, GitRef, NewRun, Pusher, RepoName, Run, RunId, RunStatus, RunTimes,
};

/// Tables and indexes, created on open.
/// Why: the table is `runs`, not the scaffold's `builds`, so an old database
/// opens cleanly; the old tables are simply ignored.
const SCHEMA: [&str; 9] = [
    "CREATE TABLE IF NOT EXISTS runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        repo TEXT NOT NULL,
        commit_hash TEXT NOT NULL,
        git_ref TEXT NOT NULL,
        pusher TEXT,
        status TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        started_at INTEGER,
        finished_at INTEGER,
        logs_expire_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS runs_status ON runs (status, id)",
    "CREATE INDEX IF NOT EXISTS runs_repo ON runs (repo, id)",
    // Why: `outputs` is space-separated; store paths never contain spaces.
    "CREATE TABLE IF NOT EXISTS checks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id INTEGER NOT NULL REFERENCES runs (id),
        attr TEXT NOT NULL,
        system TEXT NOT NULL,
        drv_path TEXT,
        outputs TEXT,
        status TEXT NOT NULL,
        machine TEXT,
        tries INTEGER NOT NULL DEFAULT 1,
        UNIQUE (run_id, attr)
    )",
    "CREATE TABLE IF NOT EXISTS log_lines (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id INTEGER NOT NULL REFERENCES runs (id),
        origin TEXT NOT NULL,
        text TEXT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS log_lines_run ON log_lines (run_id, origin, id)",
    // Why: a dependency's log is stored once, under its derivation, and
    // shown with every check that needed it.
    "CREATE TABLE IF NOT EXISTS check_derivations (
        check_id INTEGER NOT NULL REFERENCES checks (id),
        drv_path TEXT NOT NULL,
        PRIMARY KEY (check_id, drv_path)
    )",
    // Why: the outbox. `state` is pending, delivered or given_up.
    "CREATE TABLE IF NOT EXISTS deliveries (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id INTEGER NOT NULL REFERENCES runs (id),
        target_kind TEXT NOT NULL,
        target TEXT NOT NULL,
        payload TEXT NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        next_at INTEGER NOT NULL,
        state TEXT NOT NULL,
        last_error TEXT,
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS deliveries_due ON deliveries (state, next_at, id)",
];

/// Stored kind of a webhook target.
const WEBHOOK: &str = "webhook";

/// Insert pending deliveries, due `now`, inside `tx`.
async fn insert_deliveries(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    outbox: &[(Target, EventPayload)],
    now: i64,
) -> Result<(), StoreError> {
    for (target, payload) in outbox {
        let (kind, name) = match target {
            Target::Webhook(secret) => (WEBHOOK, secret.as_str()),
        };
        let json = serde_json::to_string(payload).map_err(StoreError::new)?;
        sqlx::query(
            "INSERT INTO deliveries (run_id, target_kind, target, payload, next_at, state, created_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?)",
        )
        .bind(payload.run())
        .bind(kind)
        .bind(name)
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(StoreError::new)?;
    }
    Ok(())
}

/// Columns every query reads back, in [`run_from_row`]'s order.
const COLUMNS: &str =
    "id, repo, commit_hash, git_ref, pusher, status, created_at, started_at, finished_at";

/// Check columns, as [`check_from_row`] reads them.
const CHECK_COLUMNS: &str = "id, run_id, attr, system, drv_path, outputs, status, machine, tries";

/// Columns added after a table was first released: table, column, definition.
/// Why: `CREATE TABLE IF NOT EXISTS` leaves an older table as it was.
const ADDED_COLUMNS: [(&str, &str, &str); 2] = [
    ("checks", "tries", "INTEGER NOT NULL DEFAULT 1"),
    ("runs", "logs_expire_at", "INTEGER"),
];

/// Checks of runs that are still running. Why: what a requeue drops.
const RUNNING_CHECKS: &str =
    "SELECT id FROM checks WHERE run_id IN (SELECT id FROM runs WHERE status = 'running')";

/// Statuses a check can still move from.
const OPEN_CHECKS: &str = "('queued', 'building')";

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
            for (table, column, definition) in ADDED_COLUMNS {
                let present: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
                        .bind(table)
                        .bind(column)
                        .fetch_one(&pool)
                        .await
                        .map_err(StoreError::new)?;
                if present == 0 {
                    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
                    sqlx::query(&sql)
                        .execute(&pool)
                        .await
                        .map_err(StoreError::new)?;
                }
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

/// Rebuild a check from a row selected with [`CHECK_COLUMNS`].
fn check_from_row(row: &SqliteRow) -> Result<Check, StoreError> {
    let get = |column: &str| {
        row.try_get::<Option<String>, _>(column)
            .map_err(StoreError::new)
    };
    let derivation = match (get("drv_path")?, get("outputs")?) {
        (Some(path), outputs) => {
            let outputs = outputs.as_deref().unwrap_or_default();
            Some(Derivation::new(
                path,
                outputs.split_whitespace().map(String::from).collect(),
            ))
        }
        (None, _) => None,
    };
    let status: &str = row.try_get("status").map_err(StoreError::new)?;
    let check = NewCheck::from_parts(
        row.try_get("attr").map_err(StoreError::new)?,
        row.try_get("system").map_err(StoreError::new)?,
        derivation,
        CheckStatus::parse(status).map_err(StoreError::new)?,
    );
    Ok(Check::from_parts(
        CheckId::new(row.try_get("id").map_err(StoreError::new)?),
        RunId::new(row.try_get("run_id").map_err(StoreError::new)?),
        check,
        get("machine")?,
        row.try_get("tries").map_err(StoreError::new)?,
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
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            let unlink =
                format!("DELETE FROM check_derivations WHERE check_id IN ({RUNNING_CHECKS})");
            sqlx::query(&unlink)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::new)?;
            for table in ["log_lines", "checks"] {
                let sql = format!(
                    "DELETE FROM {table}
                     WHERE run_id IN (SELECT id FROM runs WHERE status = 'running')"
                );
                sqlx::query(&sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(StoreError::new)?;
            }
            let result = sqlx::query(
                "UPDATE runs SET status = 'queued', started_at = NULL WHERE status = 'running'",
            )
            .execute(&mut *tx)
            .await
            .map_err(StoreError::new)?;
            tx.commit().await.map_err(StoreError::new)?;
            Ok(result.rows_affected())
        }
    }

    fn finish_with_outbox(
        &self,
        id: RunId,
        status: RunStatus,
        now: i64,
        outbox: &[(Target, EventPayload)],
        keep_logs_until: Option<i64>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        async move {
            if !status.is_final() {
                return Ok(false);
            }
            // Why: one transaction, so a crash never leaves a finished run
            // without its notifications, or notifications for a live run.
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            let result = sqlx::query(
                "UPDATE runs SET status = ?, finished_at = ?, logs_expire_at = ?
                 WHERE id = ? AND status = 'running'",
            )
            .bind(status.as_str())
            .bind(now)
            .bind(keep_logs_until)
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::new)?;
            if result.rows_affected() != 1 {
                return Ok(false);
            }
            insert_deliveries(&mut tx, outbox, now).await?;
            tx.commit().await.map_err(StoreError::new)?;
            Ok(true)
        }
    }

    fn delete_expired_logs(
        &self,
        now: i64,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send {
        async move {
            let expired = "SELECT id FROM runs WHERE logs_expire_at <= ?";
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            let unlink = format!(
                "DELETE FROM check_derivations
                 WHERE check_id IN (SELECT id FROM checks WHERE run_id IN ({expired}))"
            );
            sqlx::query(&unlink)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::new)?;
            let lines = format!("DELETE FROM log_lines WHERE run_id IN ({expired})");
            let deleted = sqlx::query(&lines)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::new)?;
            sqlx::query("UPDATE runs SET logs_expire_at = NULL WHERE logs_expire_at <= ?")
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::new)?;
            tx.commit().await.map_err(StoreError::new)?;
            Ok(deleted.rows_affected())
        }
    }

    fn queue_deliveries(
        &self,
        outbox: &[(Target, EventPayload)],
        now: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        async move {
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            insert_deliveries(&mut tx, outbox, now).await?;
            tx.commit().await.map_err(StoreError::new)
        }
    }

    fn due_deliveries(
        &self,
        now: i64,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Delivery>, StoreError>> + Send {
        async move {
            let rows = sqlx::query_as::<_, (i64, String, String, String, i64)>(
                "SELECT id, target_kind, target, payload, attempts FROM deliveries
                 WHERE state = 'pending' AND next_at <= ? ORDER BY id LIMIT ?",
            )
            .bind(now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::new)?;
            rows.into_iter()
                .map(|(id, kind, target, payload, attempts)| {
                    let target = match kind.as_str() {
                        WEBHOOK => Target::Webhook(target),
                        other => {
                            return Err(StoreError::new(format!("unknown target kind {other}")));
                        }
                    };
                    let payload = serde_json::from_str(&payload).map_err(StoreError::new)?;
                    let attempts = u32::try_from(attempts).map_err(StoreError::new)?;
                    Ok(Delivery::new(id, target, payload, attempts))
                })
                .collect()
        }
    }

    fn record_attempt(
        &self,
        id: i64,
        next: NextAttempt,
        error: Option<&str>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        async move {
            let (state, next_at) = match next {
                NextAttempt::Delivered => ("delivered", None),
                NextAttempt::RetryAt(at) => ("pending", Some(at)),
                NextAttempt::GiveUp => ("given_up", None),
            };
            sqlx::query(
                "UPDATE deliveries SET attempts = attempts + 1, state = ?,
                 next_at = COALESCE(?, next_at), last_error = ? WHERE id = ?",
            )
            .bind(state)
            .bind(next_at)
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StoreError::new)?;
            Ok(())
        }
    }

    fn previous_status<'a>(
        &'a self,
        repo: &'a RepoName,
        git_ref: &'a GitRef,
        before: RunId,
    ) -> impl Future<Output = Result<Option<RunStatus>, StoreError>> + Send + 'a {
        async move {
            let status: Option<String> = sqlx::query_scalar(
                "SELECT status FROM runs
                 WHERE repo = ? AND git_ref = ? AND id < ?
                   AND status IN ('passed', 'failed', 'timed_out')
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(repo.as_str())
            .bind(git_ref.as_str())
            .bind(before.get())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::new)?;
            status
                .map(|status| RunStatus::parse(&status).map_err(StoreError::new))
                .transpose()
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

    fn insert_checks(
        &self,
        run: RunId,
        checks: Vec<NewCheck>,
    ) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send {
        async move {
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            let mut inserted = Vec::with_capacity(checks.len());
            for check in checks {
                let derivation = check.derivation();
                let outputs = derivation.map(|drv| drv.outputs().join(" "));
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO checks (run_id, attr, system, drv_path, outputs, status)
                     VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
                )
                .bind(run.get())
                .bind(check.attr())
                .bind(check.system())
                .bind(derivation.map(Derivation::path))
                .bind(outputs)
                .bind(check.status().as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(StoreError::new)?;
                inserted.push(Check::inserted(CheckId::new(id), run, check));
            }
            tx.commit().await.map_err(StoreError::new)?;
            Ok(inserted)
        }
    }

    fn update_check(
        &self,
        id: CheckId,
        status: CheckStatus,
        machine: Option<&str>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send {
        async move {
            let sql = format!(
                "UPDATE checks SET status = ?, machine = COALESCE(?, machine)
                 WHERE id = ? AND status IN {OPEN_CHECKS}"
            );
            let result = sqlx::query(&sql)
                .bind(status.as_str())
                .bind(machine)
                .bind(id.get())
                .execute(&self.pool)
                .await
                .map_err(StoreError::new)?;
            Ok(result.rows_affected() == 1)
        }
    }

    fn retry_check(&self, id: CheckId) -> impl Future<Output = Result<bool, StoreError>> + Send {
        async move {
            let result = sqlx::query(
                "UPDATE checks SET status = 'queued', machine = NULL, tries = tries + 1
                 WHERE id = ? AND status = 'failed'",
            )
            .bind(id.get())
            .execute(&self.pool)
            .await
            .map_err(StoreError::new)?;
            Ok(result.rows_affected() == 1)
        }
    }

    fn close_open_checks(
        &self,
        run: RunId,
        status: CheckStatus,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send {
        async move {
            let sql = format!(
                "UPDATE checks SET status = ? WHERE run_id = ? AND status IN {OPEN_CHECKS}"
            );
            let result = sqlx::query(&sql)
                .bind(status.as_str())
                .bind(run.get())
                .execute(&self.pool)
                .await
                .map_err(StoreError::new)?;
            Ok(result.rows_affected())
        }
    }

    fn checks(&self, run: RunId) -> impl Future<Output = Result<Vec<Check>, StoreError>> + Send {
        async move {
            let sql = format!("SELECT {CHECK_COLUMNS} FROM checks WHERE run_id = ? ORDER BY id");
            let rows = sqlx::query(&sql)
                .bind(run.get())
                .fetch_all(&self.pool)
                .await
                .map_err(StoreError::new)?;
            rows.iter().map(check_from_row).collect()
        }
    }

    fn append_log(
        &self,
        run: RunId,
        lines: &[NewLogLine<'_>],
    ) -> impl Future<Output = Result<i64, StoreError>> + Send {
        async move {
            // Why: one transaction per batch; a commit per line would make
            // chatty builds wait on the disk. SQLite has one writer at a
            // time, so the batch's ids follow one another.
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            let mut first = None;
            for line in lines {
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO log_lines (run_id, origin, text) VALUES (?, ?, ?) RETURNING id",
                )
                .bind(run.get())
                .bind(line.origin().key())
                .bind(line.text())
                .fetch_one(&mut *tx)
                .await
                .map_err(StoreError::new)?;
                first.get_or_insert(id);
            }
            tx.commit().await.map_err(StoreError::new)?;
            Ok(first.unwrap_or_default())
        }
    }

    fn link_derivations(
        &self,
        links: &[(CheckId, &str)],
    ) -> impl Future<Output = Result<(), StoreError>> + Send {
        async move {
            let mut tx = self.pool.begin().await.map_err(StoreError::new)?;
            for (check, drv) in links {
                sqlx::query(
                    "INSERT OR IGNORE INTO check_derivations (check_id, drv_path) VALUES (?, ?)",
                )
                .bind(check.get())
                .bind(drv)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::new)?;
            }
            tx.commit().await.map_err(StoreError::new)
        }
    }

    fn linked_derivations(
        &self,
        check: CheckId,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        async move {
            sqlx::query_scalar("SELECT drv_path FROM check_derivations WHERE check_id = ?")
                .bind(check.get())
                .fetch_all(&self.pool)
                .await
                .map_err(StoreError::new)
        }
    }

    fn log(
        &self,
        run: RunId,
        origins: &[LogOrigin<'_>],
    ) -> impl Future<Output = Result<Vec<LogLine>, StoreError>> + Send {
        async move {
            let filter = if origins.is_empty() {
                String::new()
            } else {
                let marks = vec!["?"; origins.len()].join(", ");
                format!("AND origin IN ({marks})")
            };
            let sql = format!(
                "SELECT id, origin, text FROM log_lines WHERE run_id = ? {filter} ORDER BY id"
            );
            let mut query = sqlx::query_as::<_, (i64, String, String)>(&sql).bind(run.get());
            for origin in origins {
                query = query.bind(origin.key());
            }
            let rows = query.fetch_all(&self.pool).await.map_err(StoreError::new)?;
            Ok(rows
                .into_iter()
                .map(|(seq, origin, text)| LogLine::new(seq, origin, text))
                .collect())
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
