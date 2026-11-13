//! Garbage collection: retention, eviction, and the orphan sweep.

use std::future::Future;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};
use crate::domain::cache::CacheSpec;

/// Entries examined per index round trip.
const BATCH: u32 = 256;

/// A cache above this percentage of its quota gets evicted down…
const HIGH_WATER_PERCENT: u64 = 90;

/// …to this percentage, least-recently-read paths first.
const LOW_WATER_PERCENT: u64 = 75;

/// `percent`% of `quota`, without overflowing or rounding small quotas to 0.
fn percent_of(quota: u64, percent: u64) -> u64 {
    let scaled = u128::from(quota) * u128::from(percent) / 100;
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// What one GC pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcReport {
    expired: u64,
    evicted: u64,
    unlinked_nars: u64,
    blobs_deleted: u64,
    bytes_freed: u64,
}

impl GcReport {
    /// Store paths removed for exceeding the retention age.
    #[must_use]
    pub fn expired(&self) -> u64 {
        self.expired
    }

    /// Store paths removed to bring a cache under its quota.
    #[must_use]
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// NAR uploads removed because no narinfo ever claimed them.
    #[must_use]
    pub fn unlinked_nars(&self) -> u64 {
        self.unlinked_nars
    }

    /// Blobs deleted from the store.
    #[must_use]
    pub fn blobs_deleted(&self) -> u64 {
        self.blobs_deleted
    }

    /// Bytes reclaimed from the store.
    #[must_use]
    pub fn bytes_freed(&self) -> u64 {
        self.bytes_freed
    }
}

impl<B, I, E> Service<B, I, E>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    /// Run one GC pass over every cache, then sweep unreferenced blobs.
    /// Why: it takes no caller — only the embedding program schedules it.
    ///
    /// # Errors
    /// Fails when the index or blob store fails; a later pass resumes the work.
    pub fn collect_garbage(&self) -> impl Future<Output = Result<GcReport, ServiceError>> + Send {
        async move {
            let now = self.env.now();
            let mut report = GcReport::default();
            for spec in &self.caches {
                report.expired += self.expire(spec, now).await?;
                report.evicted += self.evict(spec).await?;
            }
            report.unlinked_nars = self
                .index
                .prune_unlinked_nars(now - self.orphan_grace_secs)
                .await?;
            for blob in self.blobs.list().await? {
                // Why: exclusive per blob, so no upload sits between its
                // commit and its index link while we decide.
                let _gate = self.commit_gate.write().await;
                if self.index.blob_referenced(blob.digest()).await? {
                    continue;
                }
                self.blobs.delete(blob.digest()).await?;
                report.blobs_deleted += 1;
                report.bytes_freed += blob.size();
            }
            Ok(report)
        }
    }

    /// Drop paths not read within the cache's retention age.
    fn expire(
        &self,
        spec: &CacheSpec,
        now: i64,
    ) -> impl Future<Output = Result<u64, ServiceError>> + Send {
        async move {
            let Some(retention) = spec.retention_secs() else {
                return Ok(0);
            };
            let cache = spec.name().as_str();
            let mut removed = 0;
            loop {
                let stale = self
                    .index
                    .stale_narinfos(cache, Some(now - retention), BATCH)
                    .await?;
                if stale.is_empty() {
                    return Ok(removed);
                }
                for hash in &stale {
                    self.index.delete_narinfo(cache, hash).await?;
                    removed += 1;
                }
            }
        }
    }

    /// Evict least-recently-read paths from a cache above its high-water mark.
    fn evict(&self, spec: &CacheSpec) -> impl Future<Output = Result<u64, ServiceError>> + Send {
        async move {
            let quota = spec.quota_bytes();
            let cache = spec.name().as_str();
            let mut used = self.index.usage(cache).await?.bytes();
            if quota == 0 || used <= percent_of(quota, HIGH_WATER_PERCENT) {
                return Ok(0);
            }
            let target = percent_of(quota, LOW_WATER_PERCENT);
            let mut removed = 0;
            while used > target {
                let oldest = self.index.stale_narinfos(cache, None, BATCH).await?;
                if oldest.is_empty() {
                    break;
                }
                for hash in &oldest {
                    used = used.saturating_sub(self.index.delete_narinfo(cache, hash).await?);
                    removed += 1;
                    if used <= target {
                        break;
                    }
                }
            }
            Ok(removed)
        }
    }
}
