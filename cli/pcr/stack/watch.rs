use std::path::PathBuf;
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NWatcher};
use tokio::sync::mpsc;

use crate::stack::config;
use crate::stack::service::ServiceManifest;

pub struct Watcher {
    tx: mpsc::Sender<ServiceManifest>,
    repo_path: PathBuf,
}

impl Watcher {
    pub fn new(tx: mpsc::Sender<ServiceManifest>, repo_path: PathBuf) -> Self {
        Self { tx, repo_path }
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

    // Event loop with debounce: wait for the first event, then sleep briefly
    // to collect any follow-up events (e.g. editor save-triggered flurries).
    while let Some(event) = event_rx.recv().await {
        if !is_relevant(&event) {
            continue;
        }

        // Debounce: wait 200ms and drain any additional events
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(event) = event_rx.try_recv() {
            if is_relevant(&event) {
                // Keep the latest event, we only need to know "something changed"
            }
        }

        // Re-parse the flake config and send the new manifest
        let Ok((raw_services, _, _)) = config::parse_stack_config(&watcher.repo_path) else {
            tracing::warn!("failed to re-parse config after file change");
            continue;
        };
        let Ok(graph) = config::ServiceGraph::from_services(raw_services) else {
            tracing::warn!("invalid service graph after file change");
            continue;
        };

        let manifest = ServiceManifest::from_graph(&graph, &watcher.repo_path);
        if watcher.tx.send(manifest).await.is_err() {
            // Supervisor has shut down, stop watching
            break;
        }
    }
}
