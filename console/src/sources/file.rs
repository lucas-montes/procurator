//! A JSON log file written by `tracing-appender` (manual runs).
//!
//! At startup the last `entries` lines are read back (approximated by
//! bytes: about 1 KiB per line), then new lines are followed. Truncation and
//! replacement are detected: the file is reopened from the start. With daily
//! rotation `tracing-appender` writes `<path>.<YYYY-MM-DD>`; when `path`
//! itself doesn't exist, the newest such file is followed and a newer one
//! is picked up when it appears.

use std::{
    io::SeekFrom,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncSeekExt, BufReader},
    sync::mpsc::Sender,
    time::sleep,
};

use super::event::{SourceEvent, line, status};

/// Bytes read back per requested bootstrap line.
const BYTES_PER_LINE: u64 = 1024;
const POLL: Duration = Duration::from_millis(250);
/// Polls between checks for rotation.
const ROTATION_CHECK_EVERY: u32 = 4;

/// Follows `path` forever.
pub async fn follow_file(source: usize, path: PathBuf, entries: u64, tx: Sender<SourceEvent>) {
    let mut first_open = true;
    loop {
        let Some(current) = resolve(&path).await else {
            status(&tx, source, Some(format!("{} not found", path.display()))).await;
            sleep(Duration::from_secs(1)).await;
            continue;
        };
        let tail = if first_open {
            entries.saturating_mul(BYTES_PER_LINE)
        } else {
            u64::MAX
        };
        match follow_one(source, &path, &current, tail, &tx).await {
            Ok(Stop::ModelGone) => return,
            Ok(Stop::Rotated) => first_open = false,
            Err(err) => {
                status(&tx, source, Some(err)).await;
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

enum Stop {
    Rotated,
    ModelGone,
}

/// Follows one concrete file until it is rotated or truncated. `tail` bytes
/// are read back from its end (`u64::MAX`: the whole file).
async fn follow_one(
    source: usize,
    configured: &Path,
    current: &Path,
    tail: u64,
    tx: &Sender<SourceEvent>,
) -> Result<Stop, String> {
    let mut file = File::open(current)
        .await
        .map_err(|err| format!("cannot open {}: {err}", current.display()))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|err| format!("cannot stat {}: {err}", current.display()))?;
    let inode = metadata.ino();
    let start = metadata.len().saturating_sub(tail);
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(|err| format!("cannot seek {}: {err}", current.display()))?;
    let mut reader = BufReader::new(file);
    let mut buffer: Vec<u8> = Vec::new();
    if start > 0 {
        // Skip the partial line we landed in.
        reader
            .read_until(b'\n', &mut buffer)
            .await
            .map_err(|err| format!("cannot read {}: {err}", current.display()))?;
        buffer.clear();
    }
    status(tx, source, None).await;

    let mut position = start;
    let mut idle_polls: u32 = 0;
    loop {
        let read = reader
            .read_until(b'\n', &mut buffer)
            .await
            .map_err(|err| format!("cannot read {}: {err}", current.display()))?;
        position = position.saturating_add(u64::try_from(read).unwrap_or(0));
        if read > 0 && buffer.last() == Some(&b'\n') {
            idle_polls = 0;
            buffer.pop();
            let text = String::from_utf8_lossy(&buffer).into_owned();
            buffer.clear();
            if !line(tx, source, text).await {
                return Ok(Stop::ModelGone);
            }
            continue;
        }
        // End of file (possibly mid-line: the rest stays in `buffer`).
        idle_polls = idle_polls.saturating_add(1);
        if idle_polls.is_multiple_of(ROTATION_CHECK_EVERY)
            && rotated(configured, current, inode, position).await
        {
            return Ok(Stop::Rotated);
        }
        sleep(POLL).await;
    }
}

/// Whether `current` was replaced, truncated, or superseded by a newer
/// rotated file.
async fn rotated(configured: &Path, current: &Path, inode: u64, position: u64) -> bool {
    match tokio::fs::metadata(current).await {
        Ok(metadata) if metadata.ino() == inode && metadata.len() >= position => {}
        _ => return true,
    }
    resolve(configured)
        .await
        .is_some_and(|newest| newest != current)
}

/// The file to follow for `configured`: itself when it exists, else the
/// newest `<name>.<suffix>` sibling written by a rotating appender.
async fn resolve(configured: &Path) -> Option<PathBuf> {
    if tokio::fs::metadata(configured).await.is_ok() {
        return Some(configured.to_path_buf());
    }
    let directory = configured
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = configured.file_name()?.to_str()?;
    let prefix = format!("{name}.");
    let mut entries = tokio::fs::read_dir(directory).await.ok()?;
    let mut newest: Option<PathBuf> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        // Date suffixes sort lexicographically.
        if file_name.starts_with(&prefix)
            && newest
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|current| current.to_str())
                .is_none_or(|current| file_name > current)
        {
            newest = Some(entry.path());
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use std::{io::Write as _, time::Duration};

    use tokio::{sync::mpsc, time::timeout};

    use super::{SourceEvent, follow_file, resolve};

    async fn next_line(rx: &mut mpsc::Receiver<SourceEvent>) -> String {
        loop {
            let event = timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("a line within 5 s")
                .expect("source alive");
            if let SourceEvent::Line { text, .. } = event {
                return text;
            }
        }
    }

    #[tokio::test]
    async fn follows_appends_and_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("svc.log");
        std::fs::write(&path, "one\ntwo\n").expect("write");
        let (tx, mut rx) = mpsc::channel(16);
        let task = tokio::spawn(follow_file(0, path.clone(), 100, tx));

        assert_eq!(next_line(&mut rx).await, "one");
        assert_eq!(next_line(&mut rx).await, "two");

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open");
        file.write_all(b"thr").expect("partial");
        file.flush().expect("flush");
        tokio::time::sleep(Duration::from_millis(300)).await;
        file.write_all(b"ee\n").expect("rest");
        assert_eq!(next_line(&mut rx).await, "three");

        std::fs::write(&path, "new\n").expect("truncate");
        assert_eq!(next_line(&mut rx).await, "new");
        task.abort();
    }

    #[tokio::test]
    async fn resolves_newest_rotated_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("w.log.2026-11-14"), "").expect("old");
        std::fs::write(dir.path().join("w.log.2026-11-15"), "").expect("new");
        let resolved = resolve(&dir.path().join("w.log")).await.expect("resolved");
        assert!(resolved.ends_with("w.log.2026-11-15"));
    }
}
