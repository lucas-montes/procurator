//! Lines of running runs, fanned out to whoever follows them.
//! Why: not a `Notifier` concern. Readers replay what is stored, then
//! receive what is appended after; the store stays the source of truth.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;

use crate::domain::log::{LogLine, NewLogLine};
use crate::domain::run::RunId;

/// Lines a slow reader may fall behind by before it reloads from the store.
const BACKLOG: usize = 4096;

/// One broadcast channel per followed run.
#[derive(Debug, Default)]
pub struct LiveLogs {
    runs: Mutex<HashMap<RunId, broadcast::Sender<LogLine>>>,
}

impl LiveLogs {
    fn runs(&self) -> MutexGuard<'_, HashMap<RunId, broadcast::Sender<LogLine>>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Receive every line appended to `run` from now on, until it is
    /// [closed](Self::close).
    pub fn subscribe(&self, run: RunId) -> broadcast::Receiver<LogLine> {
        self.runs()
            .entry(run)
            .or_insert_with(|| broadcast::channel(BACKLOG).0)
            .subscribe()
    }

    /// Hand lines just stored to `run`'s readers; the first was stored as
    /// `first`, the rest after it. Nothing is copied when nobody follows.
    pub fn publish(&self, run: RunId, first: i64, lines: &[NewLogLine<'_>]) {
        let runs = self.runs();
        let Some(sender) = runs.get(&run).filter(|sender| sender.receiver_count() > 0) else {
            return;
        };
        for (seq, line) in (first..).zip(lines) {
            let line = LogLine::new(
                seq,
                String::from(line.origin().key()),
                String::from(line.text()),
            );
            // Why: fails only when every reader left meanwhile.
            let _ = sender.send(line);
        }
    }

    /// `run` has no more lines: its readers see the end.
    pub fn close(&self, run: RunId) {
        self.runs().remove(&run);
    }
}
