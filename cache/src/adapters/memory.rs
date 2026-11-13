//! In-memory blob store and index.
//! Why: fast tests of the use cases, and embedding without disk or `SQLite`.

use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::io::{self, Cursor};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt as _};

use crate::application::ports::{
    BlobError, BlobMeta, BlobStore, Index, IndexError, NarRecord, Staged, Usage,
};
use crate::domain::access::TokenRecord;
use crate::domain::digest::Digest;

/// Blobs held in a map.
#[derive(Debug, Default)]
pub struct MemoryBlobStore {
    blobs: Mutex<HashMap<Digest, Bytes>>,
    staged: Mutex<HashMap<String, Bytes>>,
    next_id: AtomicU64,
}

impl MemoryBlobStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Lock a mutex, recovering the data if a panicking thread poisoned it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl BlobStore for MemoryBlobStore {
    type Reader = Cursor<Bytes>;

    fn stage<S>(
        &self,
        mut stream: S,
        max_bytes: u64,
    ) -> impl Future<Output = Result<Staged, BlobError>> + Send
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Unpin,
    {
        async move {
            let mut buffer = BytesMut::new();
            let mut hasher = Digest::hasher();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(BlobError::Interrupted)?;
                let total = u64::try_from(buffer.len() + chunk.len()).unwrap_or(u64::MAX);
                if total > max_bytes {
                    return Err(BlobError::TooLarge);
                }
                hasher.update(&chunk);
                buffer.extend_from_slice(&chunk);
            }
            let size = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
            let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
            lock(&self.staged).insert(id.clone(), buffer.freeze());
            Ok(Staged::new(hasher.finish(), size, id))
        }
    }

    fn commit(&self, staged: Staged) -> impl Future<Output = Result<(), BlobError>> + Send {
        let bytes = lock(&self.staged).remove(staged.id());
        ready(match bytes {
            Some(bytes) => {
                lock(&self.blobs).insert(*staged.digest(), bytes);
                Ok(())
            }
            None => Err(BlobError::Backend(String::from("unknown staged blob"))),
        })
    }

    fn discard(&self, staged: Staged) -> impl Future<Output = ()> + Send {
        lock(&self.staged).remove(staged.id());
        ready(())
    }

    fn open(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<Option<(u64, Self::Reader)>, BlobError>> + Send {
        let found = lock(&self.blobs).get(digest).map(|bytes| {
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            (size, Cursor::new(Bytes::clone(bytes)))
        });
        ready(Ok(found))
    }

    fn delete(&self, digest: &Digest) -> impl Future<Output = Result<(), BlobError>> + Send {
        lock(&self.blobs).remove(digest);
        ready(Ok(()))
    }

    fn list(&self) -> impl Future<Output = Result<Vec<BlobMeta>, BlobError>> + Send {
        let metas = lock(&self.blobs)
            .iter()
            .map(|(digest, bytes)| {
                BlobMeta::new(*digest, u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            })
            .collect();
        ready(Ok(metas))
    }
}

/// One NAR link.
#[derive(Debug)]
struct NarRow {
    nar: NarRecord,
    created_at: i64,
}

/// One stored narinfo.
#[derive(Debug)]
struct NarinfoRow {
    nar_file: String,
    body: String,
    last_access: i64,
}

/// Everything the index knows. Keys are `(cache, file | hash)`.
#[derive(Debug, Default)]
struct Tables {
    tokens: Vec<TokenRecord>,
    nars: BTreeMap<(String, String), NarRow>,
    narinfos: BTreeMap<(String, String), NarinfoRow>,
}

impl Tables {
    fn nar_in_use(&self, cache: &str, file: &str) -> bool {
        self.narinfos
            .iter()
            .any(|((c, _), row)| c == cache && row.nar_file == file)
    }
}

/// An index held in memory behind one lock.
#[derive(Debug, Default)]
pub struct MemoryIndex {
    tables: Mutex<Tables>,
}

impl MemoryIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Build an owned `(cache, key)` map key.
fn key(cache: &str, name: &str) -> (String, String) {
    (String::from(cache), String::from(name))
}

impl Index for MemoryIndex {
    fn ping(&self) -> impl Future<Output = Result<(), IndexError>> + Send {
        ready(Ok(()))
    }

    fn insert_token(
        &self,
        token: &TokenRecord,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        lock(&self.tables).tokens.push(TokenRecord::clone(token));
        ready(Ok(()))
    }

    fn token_by_hash(
        &self,
        hash: &Digest,
    ) -> impl Future<Output = Result<Option<TokenRecord>, IndexError>> + Send {
        let found = lock(&self.tables)
            .tokens
            .iter()
            .find(|t| t.hash() == hash)
            .cloned();
        ready(Ok(found))
    }

    fn tokens(&self) -> impl Future<Output = Result<Vec<TokenRecord>, IndexError>> + Send {
        ready(Ok(lock(&self.tables).tokens.clone()))
    }

    fn revoke_token(
        &self,
        id: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        let mut tables = lock(&self.tables);
        let live = tables
            .tokens
            .iter_mut()
            .find(|t| t.id() == id && t.revoked_at().is_none());
        let revoked = live.is_some_and(|token| {
            *token = TokenRecord::new(
                String::from(token.id()),
                String::from(token.name()),
                *token.hash(),
                token.is_admin(),
                token.grants().to_vec(),
                token.created_at(),
                Some(now),
            );
            true
        });
        ready(Ok(revoked))
    }

    fn link_nar(
        &self,
        cache: &str,
        file: &str,
        nar: NarRecord,
        now: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        lock(&self.tables).nars.insert(
            key(cache, file),
            NarRow {
                nar,
                created_at: now,
            },
        );
        ready(Ok(()))
    }

    fn nar(
        &self,
        cache: &str,
        file: &str,
    ) -> impl Future<Output = Result<Option<NarRecord>, IndexError>> + Send {
        let found = lock(&self.tables)
            .nars
            .get(&key(cache, file))
            .map(|r| r.nar);
        ready(Ok(found))
    }

    fn put_narinfo(
        &self,
        cache: &str,
        hash: &str,
        nar_file: &str,
        body: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        let mut tables = lock(&self.tables);
        let held = tables.nars.contains_key(&key(cache, nar_file));
        if held {
            tables.narinfos.insert(
                key(cache, hash),
                NarinfoRow {
                    nar_file: String::from(nar_file),
                    body: String::from(body),
                    last_access: now,
                },
            );
        }
        ready(Ok(held))
    }

    fn narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<Option<String>, IndexError>> + Send {
        let found = lock(&self.tables)
            .narinfos
            .get(&key(cache, hash))
            .map(|row| row.body.clone());
        ready(Ok(found))
    }

    fn touch_narinfo(
        &self,
        cache: &str,
        hash: &str,
        now: i64,
        stale_before: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send {
        if let Some(row) = lock(&self.tables).narinfos.get_mut(&key(cache, hash))
            && row.last_access < stale_before
        {
            row.last_access = now;
        }
        ready(Ok(()))
    }

    fn usage(&self, cache: &str) -> impl Future<Output = Result<Usage, IndexError>> + Send {
        let tables = lock(&self.tables);
        let (bytes, nars) = tables
            .nars
            .iter()
            .filter(|((c, _), _)| c == cache)
            .fold((0, 0), |(bytes, count), (_, row)| {
                (bytes + row.nar.size(), count + 1)
            });
        let narinfos = tables.narinfos.keys().filter(|(c, _)| c == cache).count();
        let narinfos = u64::try_from(narinfos).unwrap_or(u64::MAX);
        ready(Ok(Usage::new(bytes, nars, narinfos)))
    }

    fn stale_narinfos(
        &self,
        cache: &str,
        accessed_before: Option<i64>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>, IndexError>> + Send {
        let tables = lock(&self.tables);
        let mut rows: Vec<(i64, &str)> = tables
            .narinfos
            .iter()
            .filter(|((c, _), row)| {
                c == cache && accessed_before.is_none_or(|cutoff| row.last_access < cutoff)
            })
            .map(|((_, hash), row)| (row.last_access, hash.as_str()))
            .collect();
        rows.sort_unstable();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let hashes = rows
            .into_iter()
            .take(limit)
            .map(|(_, hash)| String::from(hash))
            .collect();
        ready(Ok(hashes))
    }

    fn delete_narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send {
        let mut tables = lock(&self.tables);
        let freed = tables.narinfos.remove(&key(cache, hash)).map_or(0, |row| {
            if tables.nar_in_use(cache, &row.nar_file) {
                return 0;
            }
            tables
                .nars
                .remove(&key(cache, &row.nar_file))
                .map_or(0, |nar| nar.nar.size())
        });
        ready(Ok(freed))
    }

    fn prune_unlinked_nars(
        &self,
        created_before: i64,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send {
        let mut tables = lock(&self.tables);
        let doomed: Vec<(String, String)> = tables
            .nars
            .iter()
            .filter(|((cache, file), row)| {
                row.created_at < created_before && !tables.nar_in_use(cache, file)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in &doomed {
            tables.nars.remove(k);
        }
        ready(Ok(u64::try_from(doomed.len()).unwrap_or(u64::MAX)))
    }

    fn blob_referenced(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send {
        let referenced = lock(&self.tables)
            .nars
            .values()
            .any(|row| row.nar.digest() == digest);
        ready(Ok(referenced))
    }
}
