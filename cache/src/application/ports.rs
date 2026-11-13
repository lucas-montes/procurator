//! Ports: what the use cases need from the outside world.
//! Why: storage, index, clock, and randomness are swappable (disk, `SQLite`,
//! memory today; S3 or Postgres later) without touching a use case.

use std::fmt;
use std::future::Future;
use std::io;

use bytes::Bytes;
use futures::Stream;
use tokio::io::AsyncRead;

use crate::domain::access::TokenRecord;
use crate::domain::digest::Digest;

/// A blob written to staging, hashed, and not yet visible.
#[derive(Debug)]
pub struct Staged {
    digest: Digest,
    size: u64,
    id: String,
}

impl Staged {
    /// Describe a staged blob. `id` is opaque to everyone but the adapter.
    #[must_use]
    pub fn new(digest: Digest, size: u64, id: String) -> Self {
        Self { digest, size, id }
    }

    /// SHA-256 of the staged bytes.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    /// Number of staged bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The adapter's handle for this staged blob.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A committed blob, as seen by a sweep.
#[derive(Debug, Clone, Copy)]
pub struct BlobMeta {
    digest: Digest,
    size: u64,
}

impl BlobMeta {
    /// Describe a committed blob.
    #[must_use]
    pub fn new(digest: Digest, size: u64) -> Self {
        Self { digest, size }
    }

    /// The blob's digest.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    /// The blob's size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// Blob store failures.
#[derive(Debug)]
pub enum BlobError {
    /// The stream exceeded the byte limit given to `stage`.
    TooLarge,
    /// The upload stream failed before it finished.
    Interrupted(io::Error),
    /// The backend failed.
    Backend(String),
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("blob exceeds the size limit"),
            Self::Interrupted(e) => write!(f, "upload interrupted: {e}"),
            Self::Backend(m) => write!(f, "blob store: {m}"),
        }
    }
}

impl std::error::Error for BlobError {}

/// Content-addressed blob storage: `sha256 -> bytes`.
pub trait BlobStore: Send + Sync + 'static {
    /// Byte source returned by [`BlobStore::open`].
    type Reader: AsyncRead + Send + Unpin + 'static;

    /// Write a stream to staging, hashing it; fail past `max_bytes`.
    /// Dropping the returned future must not leave staged bytes behind.
    fn stage<S>(
        &self,
        stream: S,
        max_bytes: u64,
    ) -> impl Future<Output = Result<Staged, BlobError>> + Send
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Unpin;

    /// Atomically publish a staged blob under its digest (idempotent).
    fn commit(&self, staged: Staged) -> impl Future<Output = Result<(), BlobError>> + Send;

    /// Throw a staged blob away.
    fn discard(&self, staged: Staged) -> impl Future<Output = ()> + Send;

    /// Open a committed blob, returning its size and a reader.
    fn open(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<Option<(u64, Self::Reader)>, BlobError>> + Send;

    /// Remove a committed blob (absent is fine).
    fn delete(&self, digest: &Digest) -> impl Future<Output = Result<(), BlobError>> + Send;

    /// Every committed blob. Why: the sweep finds blobs no cache references.
    fn list(&self) -> impl Future<Output = Result<Vec<BlobMeta>, BlobError>> + Send;
}

/// A NAR file a cache holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NarRecord {
    digest: Digest,
    size: u64,
}

impl NarRecord {
    /// Describe a held NAR file.
    #[must_use]
    pub fn new(digest: Digest, size: u64) -> Self {
        Self { digest, size }
    }

    /// Digest of the file's bytes.
    #[must_use]
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    /// File size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// What one cache currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    bytes: u64,
    nars: u64,
    narinfos: u64,
}

impl Usage {
    /// Describe a cache's contents.
    #[must_use]
    pub fn new(bytes: u64, nars: u64, narinfos: u64) -> Self {
        Self {
            bytes,
            nars,
            narinfos,
        }
    }

    /// Bytes of NAR files held.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Number of NAR files held.
    #[must_use]
    pub fn nars(&self) -> u64 {
        self.nars
    }

    /// Number of store paths (narinfos) held.
    #[must_use]
    pub fn narinfos(&self) -> u64 {
        self.narinfos
    }
}

/// Index failure (the backend's message).
#[derive(Debug)]
pub struct IndexError(String);

impl IndexError {
    /// Wrap a backend error.
    pub fn new(cause: impl fmt::Display) -> Self {
        Self(cause.to_string())
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "index: {}", self.0)
    }
}

impl std::error::Error for IndexError {}

/// The source of truth for what each cache holds and who may access it.
/// Times are unix seconds.
pub trait Index: Send + Sync + 'static {
    /// Cheap liveness probe.
    fn ping(&self) -> impl Future<Output = Result<(), IndexError>> + Send;

    /// Store a new token.
    fn insert_token(
        &self,
        token: &TokenRecord,
    ) -> impl Future<Output = Result<(), IndexError>> + Send;

    /// Look a token up by the hash of its secret (revoked ones included).
    fn token_by_hash(
        &self,
        hash: &Digest,
    ) -> impl Future<Output = Result<Option<TokenRecord>, IndexError>> + Send;

    /// Every token, oldest first.
    fn tokens(&self) -> impl Future<Output = Result<Vec<TokenRecord>, IndexError>> + Send;

    /// Mark a token revoked; `false` when no live token has that id.
    fn revoke_token(
        &self,
        id: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send;

    /// Record that `cache` holds NAR `file` (refreshes its age if present).
    fn link_nar(
        &self,
        cache: &str,
        file: &str,
        nar: NarRecord,
        now: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send;

    /// The NAR `file` held by `cache`, if any.
    fn nar(
        &self,
        cache: &str,
        file: &str,
    ) -> impl Future<Output = Result<Option<NarRecord>, IndexError>> + Send;

    /// Store a narinfo, atomically requiring `cache` to hold `nar_file`.
    /// `false` when the NAR is not held.
    fn put_narinfo(
        &self,
        cache: &str,
        hash: &str,
        nar_file: &str,
        body: &str,
        now: i64,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send;

    /// The stored narinfo body for a store hash.
    fn narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<Option<String>, IndexError>> + Send;

    /// Set last-access to `now` if it is older than `stale_before`.
    fn touch_narinfo(
        &self,
        cache: &str,
        hash: &str,
        now: i64,
        stale_before: i64,
    ) -> impl Future<Output = Result<(), IndexError>> + Send;

    /// What `cache` currently holds.
    fn usage(&self, cache: &str) -> impl Future<Output = Result<Usage, IndexError>> + Send;

    /// Store hashes least-recently accessed first, optionally only those
    /// last accessed before a cutoff.
    fn stale_narinfos(
        &self,
        cache: &str,
        accessed_before: Option<i64>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<String>, IndexError>> + Send;

    /// Delete a narinfo, and its NAR link when no other narinfo in the cache
    /// uses it. Returns the bytes the cache no longer holds.
    fn delete_narinfo(
        &self,
        cache: &str,
        hash: &str,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send;

    /// Drop NAR links older than `created_before` that no narinfo uses.
    /// Returns how many were dropped.
    fn prune_unlinked_nars(
        &self,
        created_before: i64,
    ) -> impl Future<Output = Result<u64, IndexError>> + Send;

    /// Whether any cache still links a blob.
    fn blob_referenced(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<bool, IndexError>> + Send;
}

/// Wall clock.
pub trait Clock: Send + Sync + 'static {
    /// Current time, unix seconds.
    fn now(&self) -> i64;
}

/// Cryptographic randomness.
pub trait Entropy: Send + Sync + 'static {
    /// Fill `buf` with unpredictable bytes.
    fn fill(&self, buf: &mut [u8]);
}
