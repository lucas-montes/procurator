//! What sources send to the model.

use tokio::sync::mpsc::Sender;

/// One raw line or a connection status change from source `source`
/// (index into the configured sources).
pub enum SourceEvent {
    /// A log line as the service wrote it (for journald sources: the
    /// entry's `MESSAGE`).
    Line { source: usize, text: String },
    /// The source connected, or lost its connection with a reason.
    Status {
        source: usize,
        error: Option<String>,
    },
}

/// Sends a status update; a closed model means the process is stopping.
pub async fn status(tx: &Sender<SourceEvent>, source: usize, error: Option<String>) {
    let _ = tx.send(SourceEvent::Status { source, error }).await;
}

/// Sends a line. Returns `false` when the model is gone.
pub async fn line(tx: &Sender<SourceEvent>, source: usize, text: String) -> bool {
    tx.send(SourceEvent::Line { source, text }).await.is_ok()
}
