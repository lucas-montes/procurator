//! Nix binary-cache use cases: cache info, narinfo, and NAR read/write.

use std::future::Future;
use std::io;

use bytes::Bytes;
use futures::Stream;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{BlobError, BlobStore, Clock, Entropy, Index, NarRecord};
use crate::domain::access::{Action, Principal};
use crate::domain::nix::{NarFile, Narinfo, StoreHash};

/// How stale a last-access time may get before a read refreshes it.
/// Why: reads must not turn into an index write each.
const TOUCH_INTERVAL_SECS: i64 = 3600;

/// What `/nix-cache-info` advertises.
#[derive(Debug, Clone, Copy)]
pub struct CacheInfo<'a> {
    store_dir: &'a str,
    priority: u32,
}

impl CacheInfo<'_> {
    /// Render the `nix-cache-info` body.
    #[must_use]
    pub fn render(&self) -> String {
        // Why: `WantMassQuery: 1` tells Nix it may query narinfos in parallel.
        format!(
            "StoreDir: {}\nWantMassQuery: 1\nPriority: {}\n",
            self.store_dir, self.priority
        )
    }
}

impl<B, I, E> Service<B, I, E>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    /// The cache's `nix-cache-info`.
    ///
    /// # Errors
    /// Fails when the cache is unknown or the caller may not read it.
    pub fn cache_info(&self, who: &Principal, cache: &str) -> Result<CacheInfo<'_>, ServiceError> {
        let spec = self.authorized(who, cache, Action::Read)?;
        Ok(CacheInfo {
            store_dir: spec.store_dir(),
            priority: spec.priority(),
        })
    }

    /// Fetch the narinfo for a store hash.
    ///
    /// # Errors
    /// Fails when the cache is unknown, the caller lacks access, or a port fails.
    pub fn narinfo(
        &self,
        who: &Principal,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<Option<String>, ServiceError>> + Send {
        async move {
            let spec = self.authorized(who, cache, Action::Read)?;
            let hash = StoreHash::parse(hash)?.as_str();
            let cache = spec.name().as_str();
            let body = self.index.narinfo(cache, hash).await?;
            if body.is_some() {
                let now = self.env.now();
                self.index
                    .touch_narinfo(cache, hash, now, now - TOUCH_INTERVAL_SECS)
                    .await?;
            }
            Ok(body)
        }
    }

    /// Validate, sign, and store an uploaded narinfo. Its NAR must already
    /// be in this cache.
    ///
    /// # Errors
    /// Fails when the caller may not write, the narinfo is invalid or disagrees
    /// with the stored NAR, or the NAR was never uploaded.
    pub fn put_narinfo(
        &self,
        who: &Principal,
        cache: &str,
        hash: &str,
        body: &str,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send {
        async move {
            let spec = self.authorized(who, cache, Action::Write)?;
            let hash = StoreHash::parse(hash)?;
            let info = Narinfo::parse(body, spec.store_dir())?;
            // Why: bind the URL hash to `StorePath` so one path cannot shadow another.
            if info.store_hash() != hash {
                return Err(ServiceError::Invalid("StorePath does not match the URL"));
            }
            let cache = spec.name().as_str();
            let file = info.nar_file().name();
            let nar = self
                .index
                .nar(cache, file)
                .await?
                .ok_or(ServiceError::NarNotUploaded)?;
            info.check_against_file(nar.digest(), nar.size())?;
            let signed = info.render_signed(spec.signing_key(), spec.store_dir());
            let stored = self
                .index
                .put_narinfo(cache, hash.as_str(), file, &signed, self.env.now())
                .await?;
            // Why: GC may have dropped the NAR between the lookup and the insert.
            stored.then_some(()).ok_or(ServiceError::NarNotUploaded)
        }
    }

    /// Size of a NAR file the cache holds. Why: `HEAD` without opening it.
    ///
    /// # Errors
    /// Fails when the cache is unknown, the caller lacks access, or a port fails.
    pub fn nar_size(
        &self,
        who: &Principal,
        cache: &str,
        file: &str,
    ) -> impl Future<Output = Result<Option<u64>, ServiceError>> + Send {
        async move {
            let spec = self.authorized(who, cache, Action::Read)?;
            let file = NarFile::parse(file)?;
            let nar = self.index.nar(spec.name().as_str(), file.name()).await?;
            Ok(nar.map(|n| n.size()))
        }
    }

    /// Open a NAR file for download: its size and a byte reader.
    ///
    /// # Errors
    /// Fails when the cache is unknown, the caller lacks access, or a port fails.
    pub fn open_nar(
        &self,
        who: &Principal,
        cache: &str,
        file: &str,
    ) -> impl Future<Output = Result<Option<(u64, B::Reader)>, ServiceError>> + Send {
        async move {
            let spec = self.authorized(who, cache, Action::Read)?;
            let file = NarFile::parse(file)?;
            // Why: membership comes from the index, never from the blob store —
            // a blob another cache uploaded must stay invisible here.
            let Some(nar) = self.index.nar(spec.name().as_str(), file.name()).await? else {
                return Ok(None);
            };
            let opened = self.blobs.open(nar.digest()).await?;
            if opened.is_none() {
                tracing::error!(cache, digest = %nar.digest(), "indexed blob is missing from the store");
            }
            Ok(opened)
        }
    }

    /// Store an uploaded NAR file. The bytes must hash to the file name.
    /// `declared_len` is the client's `Content-Length`, when it sent one.
    ///
    /// # Errors
    /// Fails when the caller may not write, the name is not a NAR file name, the
    /// bytes do not hash to it, or a size limit or quota is exceeded.
    pub fn put_nar<S>(
        &self,
        who: &Principal,
        cache: &str,
        file: &str,
        declared_len: Option<u64>,
        stream: S,
    ) -> impl Future<Output = Result<(), ServiceError>> + Send
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Unpin,
    {
        async move {
            let spec = self.authorized(who, cache, Action::Write)?;
            let file = NarFile::parse(file)?;
            let cache = spec.name().as_str();
            let room = self.room(cache, spec.quota_bytes()).await?;
            if declared_len.is_some_and(|len| len > self.max_nar_bytes) {
                return Err(ServiceError::TooLarge);
            }
            if room == 0 || declared_len.is_some_and(|len| len > room) {
                return Err(ServiceError::QuotaExceeded);
            }
            let limit = room.min(self.max_nar_bytes);
            let staged = match self.blobs.stage(stream, limit).await {
                Ok(staged) => staged,
                // Why: report which limit was hit — the quota or the object cap.
                Err(BlobError::TooLarge) if limit < self.max_nar_bytes => {
                    return Err(ServiceError::QuotaExceeded);
                }
                Err(e) => return Err(e.into()),
            };
            if staged.digest() != file.digest() {
                self.blobs.discard(staged).await;
                return Err(ServiceError::HashMismatch);
            }
            let nar = NarRecord::new(*staged.digest(), staged.size());
            let _gate = self.commit_gate.read().await;
            self.blobs.commit(staged).await?;
            self.index
                .link_nar(cache, file.name(), nar, self.env.now())
                .await?;
            Ok(())
        }
    }

    /// Bytes a cache may still accept (`u64::MAX` when unlimited).
    fn room(
        &self,
        cache: &str,
        quota: u64,
    ) -> impl Future<Output = Result<u64, ServiceError>> + Send {
        async move {
            if quota == 0 {
                return Ok(u64::MAX);
            }
            let used = self.index.usage(cache).await?.bytes();
            Ok(quota.saturating_sub(used))
        }
    }
}
