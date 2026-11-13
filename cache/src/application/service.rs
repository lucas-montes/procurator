//! The [`Service`]: owns the ports and exposes every use case. Each use case
//! takes the calling [`Principal`] and authorizes it before acting.
//! Why: the use cases live in child modules so they reach the private fields.

mod gc;
mod nix;
#[cfg(test)]
mod tests;
mod tokens;

use std::future::Future;

use tokio::sync::RwLock;

pub use gc::GcReport;
pub use nix::CacheInfo;
pub use tokens::NewToken;

use super::ServiceError;
use super::ports::{BlobStore, Clock, Entropy, Index, Usage};
use crate::domain::access::{Action, Principal};
use crate::domain::cache::CacheSpec;
use crate::domain::digest::Digest;

/// Largest NAR file accepted when no quota is tighter.
const DEFAULT_MAX_NAR_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// The cache application: every use case, over swappable ports.
pub struct Service<B, I, E> {
    blobs: B,
    index: I,
    env: E,
    caches: Vec<CacheSpec>,
    root_token: Option<Digest>,
    orphan_grace_secs: i64,
    max_nar_bytes: u64,
    /// Why: a blob is committed and then linked in the index; the sweep must
    /// never run between those two steps or it would delete a live blob.
    /// Uploads share the gate, the sweep takes it exclusively per blob.
    commit_gate: RwLock<()>,
}

impl<B, I, E> Service<B, I, E>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    /// Assemble the service.
    ///
    /// `root_token` is the SHA-256 of a bootstrap admin secret, if any.
    /// `orphan_grace_secs` is how long an uploaded NAR may wait for its
    /// narinfo before GC removes it.
    #[must_use]
    pub fn new(
        blobs: B,
        index: I,
        env: E,
        caches: Vec<CacheSpec>,
        root_token: Option<Digest>,
        orphan_grace_secs: i64,
    ) -> Self {
        Self {
            blobs,
            index,
            env,
            caches,
            root_token,
            orphan_grace_secs,
            max_nar_bytes: DEFAULT_MAX_NAR_BYTES,
            commit_gate: RwLock::new(()),
        }
    }

    /// Override the per-object size limit.
    #[must_use]
    pub fn with_max_nar_bytes(mut self, max_nar_bytes: u64) -> Self {
        self.max_nar_bytes = max_nar_bytes;
        self
    }

    /// The configured caches.
    #[must_use]
    pub fn caches(&self) -> &[CacheSpec] {
        &self.caches
    }

    /// Whether the index answers. Why: readiness probes.
    ///
    /// # Errors
    /// Fails when the index does not answer.
    pub fn ready(&self) -> impl Future<Output = Result<(), ServiceError>> + Send {
        async move { Ok(self.index.ping().await?) }
    }

    /// What a cache holds, for operators.
    ///
    /// # Errors
    /// Fails when the caller is not an admin or the cache is unknown.
    pub fn usage(
        &self,
        who: &Principal,
        cache: &str,
    ) -> impl Future<Output = Result<Usage, ServiceError>> + Send {
        async move {
            who.authorize_admin()?;
            let spec = self.spec(cache)?;
            Ok(self.index.usage(spec.name().as_str()).await?)
        }
    }

    /// Find a configured cache by name.
    fn spec(&self, cache: &str) -> Result<&CacheSpec, ServiceError> {
        self.caches
            .iter()
            .find(|c| c.name().as_str() == cache)
            .ok_or(ServiceError::UnknownCache)
    }

    /// Find a cache and check the caller may perform `action` on it.
    fn authorized(
        &self,
        who: &Principal,
        cache: &str,
        action: Action,
    ) -> Result<&CacheSpec, ServiceError> {
        let spec = self.spec(cache)?;
        who.authorize(spec.name(), spec.visibility(), action)?;
        Ok(spec)
    }
}
