//! `SQLite` index. Why: one file, transactional, no extra service to run.

use std::future::Future;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row as _, SqlitePool};

use crate::application::ports::{Index, IndexError, NarRecord, Usage};
use crate::domain::access::{Grant, Permission, TokenRecord};
use crate::domain::cache::CacheName;
use crate::domain::digest::Digest;

/// Tables and indexes, created on open.
const SCHEMA: [&str; 7] = [
    "CREATE TABLE IF NOT EXISTS tokens (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        hash TEXT NOT NULL UNIQUE,
        admin INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        revoked_at INTEGER
    )",
    "CREATE TABLE IF NOT EXISTS token_grants (
        token_id TEXT NOT NULL REFERENCES tokens(id) ON DELETE CASCADE,
        cache TEXT NOT NULL,
        permission TEXT NOT NULL,
        PRIMARY KEY (token_id, cache)
    )",
    "CREATE TABLE IF NOT EXISTS nars (
        cache TEXT NOT NULL,
        file TEXT NOT NULL,
        digest TEXT NOT NULL,
        size INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (cache, file)
    )",
    "CREATE TABLE IF NOT EXISTS narinfos (
        cache TEXT NOT NULL,
        hash TEXT NOT NULL,
        nar_file TEXT NOT NULL,
        body TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_access INTEGER NOT NULL,
        PRIMARY KEY (cache, hash)
    )",
    "CREATE INDEX IF NOT EXISTS nars_digest ON nars (digest)",
    "CREATE INDEX IF NOT EXISTS narinfos_access ON narinfos (cache, last_access)",
    "CREATE INDEX IF NOT EXISTS narinfos_nar ON narinfos (cache, nar_file)",
];

/// An index stored in one `SQLite` file.
#[derive(Debug)]
pub struct SqliteIndex {
    pool: SqlitePool,
}

/// `SQLite` integers are signed; sizes and counts never are.
fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Sizes beyond `i64::MAX` cannot occur; saturate rather than wrap.
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Read a stored digest column.
fn digest(row: &SqliteRow, column: &str) -> Result<Digest, IndexError> {
    let text: &str = row.try_get(column).map_err(IndexError::new)?;
    Digest::from_hex(text).map_err(IndexError::new)
}

impl SqliteIndex {
    /// Open (creating if needed) the index at `path`.
    pub fn open(path: &Path) -> impl Future<Output = Result<Self, IndexError>> + Send {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            // Why: WAL lets downloads read while an upload commits.
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        Self::connect(options, 8)
    }

    /// A private in-memory index. Why: tests of this adapter.
    #[cfg(test)]
    fn in_memory() -> impl Future<Output = Result<Self, IndexError>> + Send {
        use std::str::FromStr as _;
        async {
            let options =
                SqliteConnectOptions::from_str("sqlite::memory:").map_err(IndexError::new)?;
            // Why: each in-memory connection is its own database, so use one.
            Self::connect(options, 1).await
        }
    }

    fn connect(
        options: SqliteConnectOptions,
        connections: u32,
    ) -> impl Future<Output = Result<Self, IndexError>> + Send {
        async move {
            let pool = SqlitePoolOptions::new()
                .max_connections(connections)
                .connect_with(options)
                .await
                .map_err(IndexError::new)?;
            for statement in SCHEMA {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .map_err(IndexError::new)?;
            }
            Ok(Self { pool })
        }
    }

    /// Load a token's grants and assemble the record.
    fn token(
        &self,
        row: SqliteRow,
    ) -> impl Future<Output = Result<TokenRecord, IndexError>> + Send {
        async move {
            let id: String = row.try_get("id").map_err(IndexError::new)?;
            let grant_rows =
                sqlx::query("SELECT cache, permission FROM token_grants WHERE token_id = ?")
                    .bind(&id)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(IndexError::new)?;
            let mut grants = Vec::with_capacity(grant_rows.len());
            for grant in &grant_rows {
                let cache: &str = grant.try_get("cache").map_err(IndexError::new)?;
                let permission: &str = grant.try_get("permission").map_err(IndexError::new)?;
                grants.push(Grant::new(
                    CacheName::parse(cache).map_err(IndexError::new)?,
                    Permission::parse(permission)
                        .ok_or_else(|| IndexError::new("unknown stored permission"))?,
                ));
            }
            Ok(TokenRecord::new(
                id,
                row.try_get("name").map_err(IndexError::new)?,
                digest(&row, "hash")?,
                row.try_get("admin").map_err(IndexError::new)?,
                grants,
                row.try_get("created_at").map_err(IndexError::new)?,
                row.try_get("revoked_at").map_err(IndexError::new)?,
            ))
        }
    }
}

impl Index for SqliteIndex {
    fn ping(&self) -> impl Future<Output = Result<(), IndexError>> + Send {
        async move {
            sqlx::query("SELECT 1")
                .execute(&self.pool)
                .await
                .map_err(IndexError::new)?;
            Ok(())
        }
    }

    fn insert_token(
        &self,
        token: &TokenRecord,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        async move {
            let mut tx = self.pool.begin().await.map_err(IndexError::new)?;
            sqlx::query(
                "INSERT INTO tokens (id, name, hash, admin, created_at, revoked_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(token.id())
            .bind(token.name())
            .bind(token.hash().to_hex())
            .bind(token.is_admin())
            .bind(token.created_at())
            .bind(token.revoked_at())
            .execute(&mut *tx)
            .await
            .map_err(IndexError::new)?;
            for grant in token.grants() {
                sqlx::query(
                    "INSERT INTO token_grants (token_id, cache, permission) VALUES (?, ?, ?)",
                )
                .bind(token.id())
                .bind(grant.cache().as_str())
                .bind(grant.permission().as_str())
                .execute(&mut *tx)
                .await
                .map_err(IndexError::new)?;
            }
            tx.commit().await.map_err(IndexError::new)
        }
    }

    fn token_by_hash(
        &self,
        hash: &Digest,
    ) -> impl Future<Output = Result<Option<TokenRecord>, IndexError>> + Send {
        async move {
            let row = sqlx::query("SELECT * FROM tokens WHERE hash = ?")
                .bind(hash.to_hex())
                .fetch_optional(&self.pool)
                .await
                .map_err(IndexError::new)?;
            match row {
                Some(row) => Ok(Some(self.token(row).await?)),
                None => Ok(None),
            }
        }
    }

    fn tokens(&self) -> impl Future<Output = Result<Vec<TokenRecord>, IndexError>> + Send {
        async move {
            let rows = sqlx::query("SELECT * FROM tokens ORDER BY created_at, id")
                .fetch_all(&self.pool)
                .await
                .map_err(IndexError::new)?;
            let mut tokens = Vec::with_capacity(rows.len());
            for row in rows {
                tokens.push(self.token(row).await?);
            }
            Ok(tokens)
        }
    }

    fn revoke_token(
        &self,
        id: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        async move {
            let done =
                sqlx::query("UPDATE tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
                    .bind(now)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(IndexError::new)?;
            Ok(done.rows_affected() > 0)
        }
    }

    fn link_nar(
        &self,
        cache: &str,
        file: &str,
        nar: NarRecord,
        now: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        async move {
            sqlx::query(
                "INSERT INTO nars (cache, file, digest, size, created_at) VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT (cache, file) DO UPDATE SET created_at = excluded.created_at",
            )
            .bind(cache)
            .bind(file)
            .bind(nar.digest().to_hex())
            .bind(signed(nar.size()))
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(IndexError::new)?;
            Ok(())
        }
    }

    fn nar(
        &self,
        cache: &str,
        file: &str,
    ) -> impl Future<Output = Result<Option<NarRecord>, IndexError>> + Send {
        async move {
            let row = sqlx::query("SELECT digest, size FROM nars WHERE cache = ? AND file = ?")
                .bind(cache)
                .bind(file)
                .fetch_optional(&self.pool)
                .await
                .map_err(IndexError::new)?;
            row.map(|row| {
                let size: i64 = row.try_get("size").map_err(IndexError::new)?;
                Ok(NarRecord::new(digest(&row, "digest")?, unsigned(size)))
            })
            .transpose()
        }
    }

    fn put_narinfo(
        &self,
        cache: &str,
        hash: &str,
        nar_file: &str,
        body: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        async move {
            // Why: one statement, so the NAR cannot vanish between check and insert.
            let done = sqlx::query(
                "INSERT INTO narinfos (cache, hash, nar_file, body, created_at, last_access)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?5
                 WHERE EXISTS (SELECT 1 FROM nars WHERE cache = ?1 AND file = ?3)
                 ON CONFLICT (cache, hash) DO UPDATE SET
                    nar_file = excluded.nar_file,
                    body = excluded.body,
                    last_access = excluded.last_access",
            )
            .bind(cache)
            .bind(hash)
            .bind(nar_file)
            .bind(body)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(IndexError::new)?;
            Ok(done.rows_affected() > 0)
        }
    }

    fn narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<Option<String>, IndexError>> + Send {
        async move {
            sqlx::query_scalar("SELECT body FROM narinfos WHERE cache = ? AND hash = ?")
                .bind(cache)
                .bind(hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(IndexError::new)
        }
    }

    fn touch_narinfo(
        &self,
        cache: &str,
        hash: &str,
        now: i64,
        stale_before: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        async move {
            sqlx::query(
                "UPDATE narinfos SET last_access = ?
                 WHERE cache = ? AND hash = ? AND last_access < ?",
            )
            .bind(now)
            .bind(cache)
            .bind(hash)
            .bind(stale_before)
            .execute(&self.pool)
            .await
            .map_err(IndexError::new)?;
            Ok(())
        }
    }

    fn usage(&self, cache: &str) -> impl Future<Output = Result<Usage, IndexError>> + Send {
        async move {
            let (bytes, nars): (i64, i64) =
                sqlx::query_as("SELECT COALESCE(SUM(size), 0), COUNT(*) FROM nars WHERE cache = ?")
                    .bind(cache)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(IndexError::new)?;
            let narinfos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM narinfos WHERE cache = ?")
                .bind(cache)
                .fetch_one(&self.pool)
                .await
                .map_err(IndexError::new)?;
            Ok(Usage::new(
                unsigned(bytes),
                unsigned(nars),
                unsigned(narinfos),
            ))
        }
    }

    fn stale_narinfos(
        &self,
        cache: &str,
        accessed_before: Option<i64>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>, IndexError>> + Send {
        async move {
            sqlx::query_scalar(
                "SELECT hash FROM narinfos
                 WHERE cache = ?1 AND (?2 IS NULL OR last_access < ?2)
                 ORDER BY last_access, hash LIMIT ?3",
            )
            .bind(cache)
            .bind(accessed_before)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(IndexError::new)
        }
    }

    fn delete_narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send {
        async move {
            let mut tx = self.pool.begin().await.map_err(IndexError::new)?;
            let nar_file: Option<String> = sqlx::query_scalar(
                "DELETE FROM narinfos WHERE cache = ? AND hash = ? RETURNING nar_file",
            )
            .bind(cache)
            .bind(hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(IndexError::new)?;
            let Some(nar_file) = nar_file else {
                return Ok(0);
            };
            // Why: two store paths with identical content share one NAR file.
            let freed: Option<i64> = sqlx::query_scalar(
                "DELETE FROM nars WHERE cache = ?1 AND file = ?2
                 AND NOT EXISTS (SELECT 1 FROM narinfos WHERE cache = ?1 AND nar_file = ?2)
                 RETURNING size",
            )
            .bind(cache)
            .bind(&nar_file)
            .fetch_optional(&mut *tx)
            .await
            .map_err(IndexError::new)?;
            tx.commit().await.map_err(IndexError::new)?;
            Ok(freed.map_or(0, unsigned))
        }
    }

    fn prune_unlinked_nars(
        &self,
        created_before: i64,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send {
        async move {
            let done = sqlx::query(
                "DELETE FROM nars WHERE created_at < ? AND NOT EXISTS (
                    SELECT 1 FROM narinfos
                    WHERE narinfos.cache = nars.cache AND narinfos.nar_file = nars.file
                 )",
            )
            .bind(created_before)
            .execute(&self.pool)
            .await
            .map_err(IndexError::new)?;
            Ok(done.rows_affected())
        }
    }

    fn blob_referenced(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        async move {
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nars WHERE digest = ?)")
                .bind(digest.to_hex())
                .fetch_one(&self.pool)
                .await
                .map_err(IndexError::new)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteIndex;
    use crate::adapters::index_contract;

    #[tokio::test]
    async fn satisfies_the_index_contract() {
        index_contract::check(&SqliteIndex::in_memory().await.unwrap()).await;
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.sqlite");
        index_contract::seed(&SqliteIndex::open(&path).await.unwrap()).await;
        index_contract::check_seeded(&SqliteIndex::open(&path).await.unwrap()).await;
    }
}
