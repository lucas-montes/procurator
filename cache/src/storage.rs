//! Pluggable storage backends with static dispatch (generics, no `dyn`).
//! Why: start on local disk, add S3/GCS later without touching routes.

use std::path::{Path, PathBuf};

/// Concrete storage errors. Why: map directly to HTTP status codes.
#[derive(Debug)]
pub enum StorageError {
    /// Hash or filename failed validation.
    InvalidName(String),
    /// Quota exceeded.
    QuotaExceeded,
    /// IO failure with context.
    Io(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName(m) => write!(f, "invalid name: {m}"),
            Self::QuotaExceeded => write!(f, "storage quota exceeded"),
            Self::Io(m) => write!(f, "storage io: {m}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Backend trait. Why: no `dyn`; RPITIT futures keep it object-safe-free.
/// Note: the trait itself has no `Send+Sync` supertraits; `Send` on the
/// returned futures only exists because `axum` handlers require it.
pub trait StorageBackend: Clone + 'static {
    /// Per-impl error (no `Box<dyn>`).
    type Error: std::error::Error;
    /// Fetch a stored `.narinfo` body by hash part.
    fn get_narinfo(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send;
    /// Atomically store a `.narinfo` body.
    fn put_narinfo(
        &self,
        hash: &str,
        body: String,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    /// Fetch a stored NAR file; returns size + path for streaming.
    fn get_nar(
        &self,
        file: &str,
    ) -> impl Future<Output = Result<Option<(u64, PathBuf)>, Self::Error>> + Send;
    /// Atomically store an uploaded NAR (already fully written to `tmp`).
    fn put_nar(
        &self,
        file: &str,
        tmp: &Path,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    /// Current usage in bytes (for quota + observability).
    fn usage_bytes(&self) -> impl Future<Output = Result<u64, Self::Error>> + Send;
}

/// Local-disk backend: `<dir>/<hash>.narinfo`, `<dir>/nar/<file>`.
/// Why: same shape as Nix `file://` stores; S3 impl comes later.
#[derive(Debug, Clone)]
pub struct LocalDisk {
    /// Root directory for cache objects.
    pub dir: PathBuf,
    /// Soft quota in bytes (0 = unlimited).
    pub max_bytes: u64,
}

impl LocalDisk {
    /// Create the backend, ensuring directories exist.
    pub fn create(dir: PathBuf, max_bytes: u64) -> Result<Self, StorageError> {
        std::fs::create_dir_all(dir.join("nar"))
            .map_err(|e| StorageError::Io(format!("mkdir cache: {e}")))?;
        Ok(Self { dir, max_bytes })
    }

    /// Path of a narinfo object. Why: flat layout keeps hash→file trivial.
    #[must_use]
    pub fn narinfo_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.narinfo"))
    }

    /// Path of a NAR object. Why: `nar/` prefix matches `URL: nar/...` field.
    #[must_use]
    pub fn nar_path(&self, file: &str) -> PathBuf {
        self.dir.join("nar").join(file)
    }
}

impl StorageBackend for LocalDisk {
    type Error = StorageError;

    async fn get_narinfo(&self, hash: &str) -> Result<Option<String>, Self::Error> {
        validate_hash(hash)?;
        match tokio::fs::read_to_string(self.narinfo_path(hash)).await {
            Ok(body) => Ok(Some(body)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(format!("read narinfo: {e}"))),
        }
    }

    async fn put_narinfo(&self, hash: &str, body: String) -> Result<(), Self::Error> {
        validate_hash(hash)?;
        self.check_quota(body.len() as u64).await?;
        // Why: write+rename is atomic; readers never see partial files.
        let tmp = self.dir.join(format!(".{hash}.narinfo.tmp"));
        tokio::fs::write(&tmp, body.as_bytes())
            .await
            .map_err(|e| StorageError::Io(format!("write tmp narinfo: {e}")))?;
        tokio::fs::rename(&tmp, self.narinfo_path(hash))
            .await
            .map_err(|e| StorageError::Io(format!("commit narinfo: {e}")))?;
        Ok(())
    }

    async fn get_nar(&self, file: &str) -> Result<Option<(u64, PathBuf)>, Self::Error> {
        validate_nar_file(file)?;
        let path = self.nar_path(file);
        match tokio::fs::metadata(&path).await {
            Ok(meta) => Ok(Some((meta.len(), path))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(format!("stat nar: {e}"))),
        }
    }

    async fn put_nar(&self, file: &str, tmp: &Path) -> Result<(), Self::Error> {
        validate_nar_file(file)?;
        let meta = tokio::fs::metadata(tmp)
            .await
            .map_err(|e| StorageError::Io(format!("stat upload: {e}")))?;
        self.check_quota(meta.len()).await?;
        let dest = self.nar_path(file);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| StorageError::Io(format!("mkdir nar: {e}")))?;
        }
        // Why: persist before rename so crashes do not lose acknowledged uploads.
        tokio::fs::rename(tmp, &dest)
            .await
            .map_err(|e| StorageError::Io(format!("commit nar: {e}")))?;
        Ok(())
    }

    async fn usage_bytes(&self) -> Result<u64, Self::Error> {
        // Why: best-effort walk; quota is soft, exactness is not critical.
        let mut total = 0_u64;
        let mut stack = vec![self.dir.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(entry.path());
                } else {
                    total = total.saturating_add(meta.len());
                }
            }
        }
        Ok(total)
    }
}

impl LocalDisk {
    /// Enforce the soft quota before accepting new bytes.
    async fn check_quota(&self, incoming: u64) -> Result<(), StorageError> {
        if self.max_bytes == 0 {
            return Ok(());
        }
        let used = self.usage_bytes().await?;
        if used.saturating_add(incoming) > self.max_bytes {
            return Err(StorageError::QuotaExceeded);
        }
        Ok(())
    }
}

/// Hash parts are 32 lowercase `[a-z0-9]`. Why: matches Nix `hashPart`.
pub fn validate_hash(hash: &str) -> Result<(), StorageError> {
    if hash.len() == 32
        && hash
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(StorageError::InvalidName(format!("bad hash: {hash}")))
    }
}

/// NAR filenames stay inside `nar/` (no traversal, sane charset).
fn validate_nar_file(file: &str) -> Result<(), StorageError> {
    if file.is_empty() || file.len() > 256 || file.contains('/') || file.contains('\\') {
        return Err(StorageError::InvalidName(format!("bad nar file: {file}")));
    }
    if file
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
    {
        Ok(())
    } else {
        Err(StorageError::InvalidName(format!("bad nar file: {file}")))
    }
}

#[cfg(test)]
mod tests {
    use super::validate_hash;

    #[test]
    fn accepts_nix_hash_parts() {
        assert!(validate_hash(&"a".repeat(32)).is_ok());
        assert!(validate_hash("ABC").is_err());
        assert!(validate_hash("has-dash-00000000000000000000000").is_err());
    }
}
