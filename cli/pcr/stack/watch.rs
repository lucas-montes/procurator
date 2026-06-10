//! Debounced recursive directory watcher.
//!
//! Used by the watch-mode loop (T04) to detect source-file changes. Some items
//! appear dead until T04 is implemented — dead-code warnings are suppressed
//! intentionally.

#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// A debounced notification that files changed in one or more watched directories.
#[derive(Debug, Clone)]
pub struct DebouncedEvent {
    /// Paths that changed since the last event.
    pub paths: Vec<PathBuf>,
}

/// Start watching `paths` recursively.
///
/// Returns a receiver that yields a [`DebouncedEvent`] after each 500 ms quiet
/// period following the last file-system event. Access (open/close/read)
/// events are filtered out to reduce noise.
///
/// The watcher runs until the returned receiver is dropped.
pub async fn watch_dirs(paths: Vec<PathBuf>) -> Result<mpsc::Receiver<DebouncedEvent>, String> {
    // ── Raw notify channel (std mpsc, used by the watcher callback) ──
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(raw_tx)
        .map_err(|e| format!("failed to create file watcher: {}", e))?;

    for path in &paths {
        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch {:?}: {}", path, e))?;
    }

    // ── Bridge: blocking std::mpsc → async tokio::mpsc ──
    let (bridge_tx, bridge_rx) = mpsc::channel::<Event>(1024);

    tokio::task::spawn_blocking(move || {
        while let Ok(Ok(event)) = raw_rx.recv() {
            if bridge_tx.blocking_send(event).is_err() {
                break; // receiver dropped
            }
        }
    });

    // ── Debounce task (fully async) ──
    let (debounced_tx, debounced_rx) = mpsc::channel::<DebouncedEvent>(16);

    tokio::spawn(async move {
        let _watcher = watcher; // keep alive
        debounce_loop(bridge_rx, debounced_tx).await;
    });

    Ok(debounced_rx)
}

/// Core debounce loop: receive events from the bridge, wait for 500 ms of
/// silence, then send accumulated paths through the tokio channel.
async fn debounce_loop(
    mut bridge_rx: mpsc::Receiver<Event>,
    debounced_tx: mpsc::Sender<DebouncedEvent>,
) {
    let debounce = Duration::from_millis(500);
    let mut pending: Vec<PathBuf> = Vec::new();

    loop {
        // Wait for the first event.
        let event = match bridge_rx.recv().await {
            Some(e) => e,
            None => break,
        };

        collect_paths(&event, &mut pending);

        // Inner debounce window: keep collecting until 500 ms of silence.
        loop {
            tokio::select! {
                Some(event) = bridge_rx.recv() => {
                    collect_paths(&event, &mut pending);
                    // Loop back — this implicitly resets the timer.
                }
                _ = tokio::time::sleep(debounce) => {
                    // 500 ms quiet — send the batch.
                    break;
                }
            }
        }

        // Send accumulated paths.
        let paths = std::mem::take(&mut pending);
        if debounced_tx.send(DebouncedEvent { paths }).await.is_err() {
            break; // receiver dropped
        }
    }
}

/// Add paths from an event to the pending set, skipping access-only events.
fn collect_paths(event: &Event, pending: &mut Vec<PathBuf>) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for path in &event.paths {
        if !pending.contains(path) {
            pending.push(path.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_watch_dirs_receives_event_on_file_create() {
        let dir = TempDir::new().unwrap();
        let watch_path = dir.path().to_path_buf();

        let mut rx = watch_dirs(vec![watch_path])
            .await
            .expect("watch_dirs should succeed");

        // Give the watcher a moment to initialise, then create a file.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, "world").unwrap();

        // Wait for the debounced event (should arrive within 3 s).
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timeout while waiting for debounced event")
            .expect("channel should not be closed");

        assert!(
            event.paths.iter().any(|p| p.ends_with("hello.txt")),
            "expected hello.txt in paths, got: {:?}",
            event.paths,
        );
    }

    #[tokio::test]
    async fn test_watch_dirs_stays_quiet_without_changes() {
        let dir = TempDir::new().unwrap();
        let watch_path = dir.path().to_path_buf();

        let mut rx = watch_dirs(vec![watch_path])
            .await
            .expect("watch_dirs should succeed");

        // Without any file changes, nothing should arrive within 800 ms.
        let result = tokio::time::timeout(Duration::from_millis(800), rx.recv()).await;
        assert!(
            matches!(result, Err(_)),
            "expected timeout, got event: {:?}",
            result
        );
    }
}
