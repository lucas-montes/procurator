//! Debounced recursive directory watcher and watch-mode lifecycle loop.
//!
//! The two top-level functions are:
//!
//! - [`watch_dirs`] — start a debounced watcher on several directories (recursive).
//! - [`watch_file`] — start a debounced watcher on a single file's parent (non-recursive),
//!   used for monitoring `flake.nix` without watching the entire repo tree.
//! - [`run_watch_loop`] — the combined file-watch + signal foreground loop
//!   that powers hot-reload mode.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::stack::logging::{color_for, colored_prefix};
use crate::stack::parser::{
    ServiceChange, ServiceGraph, WatchConfig, diff_graphs, parse_stack_config,
};
use crate::stack::process::{GRACEFUL_TIMEOUT, ProcessSupervisor};
use crate::stack::supervisor::{ServiceHandle, StackState};

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

/// Start watching a single *directory* non-recursively (used for `flake.nix`).
///
/// Only events affecting files directly inside `dir` (not subdirectories) will
/// be reported. The caller is responsible for filtering paths that match the
/// target file name.
pub async fn watch_file(dir: PathBuf) -> Result<mpsc::Receiver<DebouncedEvent>, String> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(raw_tx)
        .map_err(|e| format!("failed to create file watcher: {}", e))?;

    watcher
        .watch(&dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("failed to watch {:?}: {}", dir, e))?;

    // Bridge: blocking → async
    let (bridge_tx, bridge_rx) = mpsc::channel::<Event>(1024);
    tokio::task::spawn_blocking(move || {
        while let Ok(Ok(event)) = raw_rx.recv() {
            if bridge_tx.blocking_send(event).is_err() {
                break;
            }
        }
    });

    // Debounce
    let (debounced_tx, debounced_rx) = mpsc::channel::<DebouncedEvent>(16);
    tokio::spawn(async move {
        let _watcher = watcher;
        debounce_loop(bridge_rx, debounced_tx).await;
    });

    Ok(debounced_rx)
}

// ---------------------------------------------------------------------------
// Watch-mode lifecycle loop
// ---------------------------------------------------------------------------

/// Enter the foreground file-watch + signal loop.
///
/// Called when `stack.watch.enable = true`.  Monitors source directories for
/// changes (and optionally `flake.nix`), hot-reloads affected services, and
/// handles SIGINT / SIGTERM for graceful shutdown.
pub async fn run_watch_loop<S: StackState>(
    supervisor: &ProcessSupervisor<S>,
    mut graph: ServiceGraph,
    mut handles: HashMap<String, ServiceHandle>,
    watch_cfg: WatchConfig,
) -> Result<(), String> {
    let stack_pid = std::process::id();
    let started_at = chrono::Utc::now().to_rfc3339();

    // ── 1. Build path-to-service map ──────────────────────────────────────
    let mut path_to_services: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for (name, svc) in &graph.services {
        if let Some(src) = &svc.src {
            let full_path = if PathBuf::from(src).is_absolute() {
                PathBuf::from(src)
            } else {
                supervisor.repo_root.join(src)
            };
            // Canonicalize so that changes on symlinked or mounted paths match.
            let canonical = full_path.canonicalize().unwrap_or(full_path);
            path_to_services
                .entry(canonical)
                .or_default()
                .push(name.clone());
        }
    }

    let watched_src_dirs: Vec<PathBuf> = path_to_services.keys().cloned().collect();

    // ── 2. Start file watchers ────────────────────────────────────────────
    let mut src_rx = if !watched_src_dirs.is_empty() {
        Some(watch_dirs(watched_src_dirs).await?)
    } else {
        None
    };

    let mut flake_rx = if watch_cfg.watch_flake {
        let flake_path = supervisor.repo_root.join("flake.nix");
        if flake_path.exists() {
            Some(watch_file(supervisor.repo_root.clone()).await?)
        } else {
            None
        }
    } else {
        None
    };

    // ── 3. Install signal handlers ───────────────────────────────────────
    let mut sigint =
        signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT handler: {}", e))?;
    let mut sigterm =
        signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM handler: {}", e))?;

    println!("Watch mode enabled — listening for source changes.");

    // ── 4. Main select loop ──────────────────────────────────────────────
    loop {
        tokio::select! {
            // ── Signal branch ────────────────────────────────────────────
            _ = sigint.recv() => {
                println!("\nSIGINT received, shutting down...");
                break;
            }
            _ = sigterm.recv() => {
                println!("\nSIGTERM received, shutting down...");
                break;
            }

            // ── Source-change branch ─────────────────────────────────────
            Some(event) = async {
                match src_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let mut affected: Vec<String> = Vec::new();
                for changed in &event.paths {
                    for (watch_dir, names) in &path_to_services {
                        if changed.starts_with(watch_dir) {
                            for name in names {
                                if !affected.contains(name) {
                                    affected.push(name.clone());
                                }
                            }
                        }
                    }
                }

                if affected.is_empty() {
                    continue;
                }

                // Filter out oneShot services — they are never re-run.
                let (to_restart, skipped): (Vec<_>, Vec<_>) = affected.iter().partition(|n| {
                    !graph.services.get(*n).map(|s| s.one_shot.unwrap_or(false)).unwrap_or(true)
                });

                for name in &skipped {
                    let color = color_for(name);
                    println!(
                        "{} oneShot — source change ignored",
                        colored_prefix(name, color)
                    );
                }

                if !to_restart.is_empty() {
                    let names_refs: Vec<String> = to_restart.iter().map(|s| (*s).clone()).collect();
                    supervisor
                        .spawn_many(&names_refs, &graph, &mut handles, true, stack_pid, &started_at)
                        .await?;

                    for name in &to_restart {
                        let color = color_for(name);
                        println!(
                            "{} restarted by source change",
                            colored_prefix(name, color)
                        );
                    }
                }
            }

            // ── Flake-change branch ──────────────────────────────────────
            Some(_event) = async {
                match flake_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                // Verify the event really touched flake.nix.
                let flake_path = supervisor.repo_root.join("flake.nix");
                if !flake_path.exists() {
                    continue;
                }

                // Re-parse the flake.  If evaluation fails, warn and keep
                // the current graph.
                let (raw_services, _log_cfg, _watch_cfg) = match parse_stack_config(&supervisor.repo_root) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("flake re-parse failed (keeping current graph): {}", e);
                        continue;
                    }
                };
                let new_graph = match ServiceGraph::from_services(raw_services) {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("flake re-parse graph invalid (keeping current graph): {}", e);
                        continue;
                    }
                };

                let diff = diff_graphs(&graph, &new_graph);

                // --- CHANGED: stop + re-spawn ---
                let changed: Vec<String> = diff.iter()
                    .filter(|(_, c)| **c == ServiceChange::Changed)
                    .map(|(n, _)| n.clone())
                    .collect();
                if !changed.is_empty() {
                    supervisor
                        .spawn_many(&changed, &new_graph, &mut handles, true, stack_pid, &started_at)
                        .await?;
                    for name in &changed {
                        let color = color_for(name);
                        let is_one_shot = new_graph.services.get(name)
                            .map(|s| s.one_shot.unwrap_or(false))
                            .unwrap_or(false);
                        if is_one_shot {
                            println!("{} oneShot cmd changed — ignored in watch mode", colored_prefix(name, color));
                        } else {
                            println!("{} restarted by flake change", colored_prefix(name, color));
                        }
                    }
                }

                // --- ADDED: spawn new ---
                let added: Vec<String> = diff.iter()
                    .filter(|(_, c)| **c == ServiceChange::Added)
                    .map(|(n, _)| n.clone())
                    .collect();
                if !added.is_empty() {
                    supervisor
                        .spawn_many(&added, &new_graph, &mut handles, false, stack_pid, &started_at)
                        .await?;
                    for name in &added {
                        let color = color_for(name);
                        println!("{} added by flake change", colored_prefix(name, color));
                    }
                }

                // --- REMOVED: stop + remove from map ---
                let removed: Vec<String> = diff.iter()
                    .filter(|(_, c)| **c == ServiceChange::Removed)
                    .map(|(n, _)| n.clone())
                    .collect();
                for name in &removed {
                    if let Some(mut h) = handles.remove(name) {
                        h.stop(GRACEFUL_TIMEOUT);
                        let color = color_for(&h.name);
                        println!("{} removed by flake change", colored_prefix(name, color));
                    }
                }
                if !removed.is_empty() {
                    supervisor.persist_handles(&handles, stack_pid, &started_at)?;
                }

                // Remember the new graph for the next diff.
                graph = new_graph;
            }
        }
    }

    // ── 5. Graceful shutdown ─────────────────────────────────────────────
    for h in handles.values_mut() {
        h.stop(GRACEFUL_TIMEOUT);
    }
    supervisor.clear_state().ok();
    println!("Stack stopped.");
    Ok(())
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
