use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NWatcher};
use tokio::sync::mpsc;

use crate::stack::config;
use crate::stack::service::ServiceManifest;

/// Events produced by the file watcher and consumed by the Supervisor.
pub enum WatchEvent {
    /// Flake config changed — re-parse, diff, and restart affected services.
    ConfigChanged(ServiceManifest),
    /// Source files changed for specific services — restart them with existing cmd.
    SourceChanged(Vec<String>),
}

pub struct Watcher {
    tx: mpsc::Sender<WatchEvent>,
    repo_path: PathBuf,
    /// Map from source directory → service names that share it.
    /// Services without a `src` field are absent from this map.
    src_dirs: HashMap<PathBuf, Vec<String>>,
}

impl Watcher {
    pub fn new(
        tx: mpsc::Sender<WatchEvent>,
        repo_path: PathBuf,
        src_dirs: HashMap<PathBuf, Vec<String>>,
    ) -> Self {
        Self {
            tx,
            repo_path,
            src_dirs,
        }
    }

    /// Spawn the watcher loop. Watches the repo root recursively for file
    /// changes, re-parses the config, and sends a new `ServiceManifest`
    /// to the supervisor.
    pub fn spawn(self, size: usize) -> tokio::task::JoinHandle<()> {
        tokio::spawn(watcher_loop(self, size))
    }
}

/// Returns true if the notify event is relevant to services (file changes).
fn is_relevant(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Returns the service names whose source directory contains `path`.
fn match_source_dirs<'a>(path: &Path, src_dirs: &'a HashMap<PathBuf, Vec<String>>) -> Vec<&'a str> {
    let mut names: Vec<&str> = Vec::new();
    for (src_dir, svc_names) in src_dirs {
        if path.starts_with(src_dir) {
            names.extend(svc_names.iter().map(|s| s.as_str()));
        }
    }
    names.sort();
    names.dedup();
    names
}

async fn watcher_loop(watcher: Watcher, size: usize) {
    // Bridge from notify's callback thread → tokio async channel
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(size);

    let mut nwatcher = match RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // Non-blocking send; if the channel is full we drop the event.
                // The next change will trigger another reload anyway.
                let _ = event_tx.try_send(event);
            }
        },
        Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "failed to create file watcher");
            return;
        }
    };

    // Watch the repo root recursively for flake/config changes
    if let Err(e) = nwatcher.watch(&watcher.repo_path, RecursiveMode::Recursive) {
        tracing::warn!(
            path = %watcher.repo_path.display(),
            error = %e,
            "failed to watch repo path"
        );
    }

    tracing::info!(
        src_dirs = ?watcher.src_dirs.iter().map(|(p, n)| format!("{}→{:?}", p.display(), n)).collect::<Vec<_>>(),
        "watcher started"
    );

    // Event loop with debounce: wait for the first event, then sleep briefly
    // to collect any follow-up events (e.g. editor save-triggered flurries).
    while let Some(event) = event_rx.recv().await {
        if !is_relevant(&event) {
            continue;
        }

        // Collect changed paths from this event batch
        let mut changed_paths: Vec<PathBuf> = event.paths.clone();

        // Debounce: wait 200ms and drain any additional events
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(event) = event_rx.try_recv() {
            if is_relevant(&event) {
                changed_paths.extend(event.paths);
            }
        }

        // Classify each changed path: source dir → collect affected services,
        // anything else → full config re-parse.
        tracing::info!(num_paths = changed_paths.len(), paths = ?changed_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), "classifying changed paths");
        let mut affected_names: Vec<String> = Vec::new();
        let mut needs_config_reparse = false;

        for path in &changed_paths {
            let src_names = match_source_dirs(path, &watcher.src_dirs);
            if !src_names.is_empty() {
                affected_names.extend(src_names.into_iter().map(|s| s.to_string()));
            } else {
                needs_config_reparse = true;
            }
        }

        affected_names.sort();
        affected_names.dedup();

        // Send source-change events first, then config-change.
        if !affected_names.is_empty() {
            if watcher
                .tx
                .send(WatchEvent::SourceChanged(affected_names))
                .await
                .is_err()
            {
                break;
            }
        }

        if needs_config_reparse {
            let Ok((raw_services, _, _)) = config::parse_stack_config(&watcher.repo_path) else {
                tracing::warn!("failed to re-parse config after file change");
                continue;
            };
            let Ok(graph) = config::ServiceGraph::from_services(raw_services) else {
                tracing::warn!("invalid service graph after file change");
                continue;
            };

            let manifest = ServiceManifest::from_graph(&graph, &watcher.repo_path);
            if watcher
                .tx
                .send(WatchEvent::ConfigChanged(manifest))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}
