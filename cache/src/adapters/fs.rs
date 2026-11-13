//! Local-disk blob store: `<root>/blobs/<aa>/<sha256 hex>`, staged under
//! `<root>/staging/`. Why: rename within one filesystem makes commits atomic.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use futures::{Stream, StreamExt as _};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt as _;

use crate::application::ports::{BlobError, BlobMeta, BlobStore, Staged};
use crate::domain::digest::Digest;

/// Blobs on a local filesystem.
#[derive(Debug)]
pub struct FsBlobStore {
    blobs: PathBuf,
    staging: PathBuf,
}

/// Wrap an IO failure with what was being done.
fn backend(what: &str, error: &io::Error) -> BlobError {
    BlobError::Backend(format!("{what}: {error}"))
}

/// Removes a staging file unless the upload completed.
/// Why: a client that disconnects mid-upload drops the future; without this
/// the partial file would stay until the next restart.
struct StagingGuard<'a> {
    path: &'a Path,
    keep: bool,
}

impl Drop for StagingGuard<'_> {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

impl FsBlobStore {
    /// Open (creating if needed) a store under `root`, discarding staging
    /// files left by a previous run.
    ///
    /// # Errors
    /// Returns [`BlobError::Backend`] when the directories cannot be prepared.
    pub fn open(root: &Path) -> Result<Self, BlobError> {
        let blobs = root.join("blobs");
        let staging = root.join("staging");
        std::fs::create_dir_all(&blobs).map_err(|e| backend("create blobs dir", &e))?;
        match std::fs::remove_dir_all(&staging) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(backend("clear staging dir", &e));
            }
            _ => {}
        }
        std::fs::create_dir_all(&staging).map_err(|e| backend("create staging dir", &e))?;
        Ok(Self { blobs, staging })
    }

    /// Where a committed blob lives. Why: fan out so no directory grows huge.
    fn blob_path(&self, digest: &Digest) -> PathBuf {
        let hex = digest.to_hex();
        self.blobs.join(&hex[..2]).join(hex)
    }
}

impl BlobStore for FsBlobStore {
    type Reader = File;

    fn stage<S>(
        &self,
        mut stream: S,
        max_bytes: u64,
    ) -> impl Future<Output = Result<Staged, BlobError>> + Send
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Unpin,
    {
        async move {
            // Why: a unique name per upload — two clients pushing the same
            // NAR must never write into one file.
            let id = uuid::Uuid::now_v7().simple().to_string();
            let path = self.staging.join(&id);
            let mut guard = StagingGuard {
                path: &path,
                keep: false,
            };
            let mut file = File::create(&path)
                .await
                .map_err(|e| backend("create staging file", &e))?;
            let mut hasher = Digest::hasher();
            let mut size = 0_u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(BlobError::Interrupted)?;
                size = size.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
                if size > max_bytes {
                    return Err(BlobError::TooLarge);
                }
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|e| backend("write staging file", &e))?;
            }
            // Why: durable before the rename, so an acknowledged upload survives a crash.
            file.sync_all()
                .await
                .map_err(|e| backend("sync staging file", &e))?;
            guard.keep = true;
            Ok(Staged::new(hasher.finish(), size, id))
        }
    }

    fn commit(&self, staged: Staged) -> impl Future<Output = Result<(), BlobError>> + Send {
        async move {
            let from = self.staging.join(staged.id());
            let to = self.blob_path(staged.digest());
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| backend("create blob dir", &e))?;
            }
            fs::rename(&from, &to)
                .await
                .map_err(|e| backend("commit blob", &e))
        }
    }

    fn discard(&self, staged: Staged) -> impl Future<Output = ()> + Send {
        async move {
            let _ = fs::remove_file(self.staging.join(staged.id())).await;
        }
    }

    fn open(
        &self,
        digest: &Digest,
    ) -> impl Future<Output = Result<Option<(u64, Self::Reader)>, BlobError>> + Send {
        async move {
            let file = match File::open(self.blob_path(digest)).await {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(backend("open blob", &e)),
            };
            let size = file
                .metadata()
                .await
                .map_err(|e| backend("stat blob", &e))?
                .len();
            Ok(Some((size, file)))
        }
    }

    fn delete(&self, digest: &Digest) -> impl Future<Output = Result<(), BlobError>> + Send {
        async move {
            match fs::remove_file(self.blob_path(digest)).await {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(backend("delete blob", &e)),
                _ => Ok(()),
            }
        }
    }

    fn list(&self) -> impl Future<Output = Result<Vec<BlobMeta>, BlobError>> + Send {
        async move {
            let mut metas = Vec::new();
            let mut shards = fs::read_dir(&self.blobs)
                .await
                .map_err(|e| backend("list blobs", &e))?;
            while let Some(shard) = shards
                .next_entry()
                .await
                .map_err(|e| backend("list blobs", &e))?
            {
                let Ok(mut entries) = fs::read_dir(shard.path()).await else {
                    continue;
                };
                while let Some(entry) = entries
                    .next_entry()
                    .await
                    .map_err(|e| backend("list blobs", &e))?
                {
                    // Why: skip anything that is not one of our blobs.
                    let name = entry.file_name();
                    let Some(digest) = name.to_str().and_then(|n| Digest::from_hex(n).ok()) else {
                        continue;
                    };
                    let Ok(meta) = entry.metadata().await else {
                        continue;
                    };
                    metas.push(BlobMeta::new(digest, meta.len()));
                }
            }
            Ok(metas)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use bytes::Bytes;
    use futures::stream;
    use tokio::io::AsyncReadExt as _;

    use super::FsBlobStore;
    use crate::application::ports::{BlobError, BlobStore};
    use crate::domain::digest::Digest;

    fn chunks(parts: &[&'static [u8]]) -> impl futures::Stream<Item = io::Result<Bytes>> + Unpin {
        stream::iter(
            parts
                .iter()
                .map(|p| Ok(Bytes::from_static(p)))
                .collect::<Vec<_>>(),
        )
    }

    fn staged_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir.join("staging")).unwrap().count()
    }

    #[tokio::test]
    async fn staged_bytes_round_trip_after_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::open(dir.path()).unwrap();
        let staged = store.stage(chunks(&[b"hel", b"lo"]), 100).await.unwrap();
        let digest = *staged.digest();
        assert_eq!(digest, Digest::of(b"hello"));
        assert_eq!(staged.size(), 5);
        assert!(store.open(&digest).await.unwrap().is_none());
        store.commit(staged).await.unwrap();

        let (size, mut reader) = store.open(&digest).await.unwrap().unwrap();
        let mut body = Vec::new();
        reader.read_to_end(&mut body).await.unwrap();
        assert_eq!((size, body.as_slice()), (5, b"hello".as_slice()));
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(staged_files(dir.path()), 0);

        store.delete(&digest).await.unwrap();
        store.delete(&digest).await.unwrap();
        assert!(store.open(&digest).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_uploads_leave_no_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::open(dir.path()).unwrap();
        let too_big = store.stage(chunks(&[b"hello", b"world"]), 7).await;
        assert!(matches!(too_big, Err(BlobError::TooLarge)));

        let broken = stream::iter(vec![
            Ok(Bytes::from_static(b"part")),
            Err(io::Error::other("client went away")),
        ]);
        assert!(matches!(
            store.stage(broken, 100).await,
            Err(BlobError::Interrupted(_))
        ));

        let discarded = store.stage(chunks(&[b"x"]), 100).await.unwrap();
        store.discard(discarded).await;
        assert_eq!(staged_files(dir.path()), 0);
    }

    #[tokio::test]
    async fn reopening_clears_stale_staging_but_keeps_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::open(dir.path()).unwrap();
        let kept = store.stage(chunks(&[b"kept"]), 100).await.unwrap();
        let digest = *kept.digest();
        store.commit(kept).await.unwrap();
        let _abandoned = store.stage(chunks(&[b"abandoned"]), 100).await.unwrap();
        assert_eq!(staged_files(dir.path()), 1);

        let reopened = FsBlobStore::open(dir.path()).unwrap();
        assert_eq!(staged_files(dir.path()), 0);
        assert!(reopened.open(&digest).await.unwrap().is_some());
    }
}
