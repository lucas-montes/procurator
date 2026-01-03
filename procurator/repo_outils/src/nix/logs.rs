use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::num::NonZeroU64;
use std::ops::Not;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    ProcessFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    JsonParseError(serde_json::Error),
    InvalidFlakePath(String),
    Timeout(Duration),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::IoError(err) => write!(f, "IO error: {}", err),
            Error::ProcessFailed { exit_code, stderr } => {
                write!(
                    f,
                    "Nix process failed with exit code {:?}: {}",
                    exit_code, stderr
                )
            }
            Error::JsonParseError(err) => write!(f, "JSON parse error: {}", err),
            Error::InvalidFlakePath(path) => write!(f, "Invalid flake path: {}", path),
            Error::Timeout(duration) => write!(f, "Process timed out after {:?}", duration),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::IoError(err) => Some(err),
            Error::JsonParseError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::IoError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::JsonParseError(err)
    }
}
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Copy, Clone, PartialOrd, Ord)]
struct EntryId(NonZeroU64);

impl EntryId {
    fn new(id: u64) -> Option<Self> {
        NonZeroU64::new(id).map(EntryId)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartEntry {
    id: u64,
    level: u8,
    parent: u64,
    text: String,
    #[serde(rename = "type")]
    log_type: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StopEntry {
    id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgEntry {
    level: u8,
    msg: String,
    raw_msg: Option<String>,
    column: Option<u32>,
    file: Option<String>,
    line: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResultEntry {
    id: u64,
    fields: Vec<u64>,
    #[serde(rename = "type")]
    log_type: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum LogEntry {
    Start(StartEntry),
    Stop(StopEntry),
    Msg(MsgEntry),
    Result(ResultEntry),
}

/// Trait for handling Nix log parsing with different strategies
pub trait Parser: Default {
    /// The output type produced by this handler
    type Output: Serialize;

    /// Process a start entry
    fn handle_start(&mut self, start: StartEntry, timestamp: SystemTime);

    /// Process a stop entry
    fn handle_stop(&mut self, stop: StopEntry, timestamp: SystemTime);

    /// Process a message entry
    fn handle_msg(&mut self, msg: MsgEntry, timestamp: SystemTime);

    /// Process a result entry
    fn handle_result(&mut self, result: ResultEntry, timestamp: SystemTime) {
        let _ = (result, timestamp); // Default: ignore
    }

    /// Process any log entry
    fn process_entry(&mut self, entry: LogEntry, timestamp: SystemTime) {
        match entry {
            LogEntry::Start(start) => self.handle_start(start, timestamp),
            LogEntry::Stop(stop) => self.handle_stop(stop, timestamp),
            LogEntry::Msg(msg) => self.handle_msg(msg, timestamp),
            LogEntry::Result(result) => self.handle_result(result, timestamp),
        }
    }

    async fn parse_lines(&mut self, reader: impl AsyncBufReadExt + Unpin) {
        let mut lines = reader.lines();

        while let Ok(Some(raw_line)) = lines.next_line().await {
            if let Some(json_part) = raw_line.strip_prefix("@nix ") {
                let timestamp = SystemTime::now();

                match serde_json::from_str::<LogEntry>(json_part) {
                    Ok(entry) => self.process_entry(entry, timestamp),
                    Err(err) => {
                        warn!(%raw_line, error=%err, "Failed to parse nix log entry");
                    }
                }
            }
        }
    }

    /// Finalize and produce output
    fn into_output(self, started_at: SystemTime, completed_at: SystemTime) -> Self::Output;
}

#[derive(Debug)]
struct Message {
    entry: MsgEntry,
    timestamp: SystemTime,
}

/// Represents a step that is currently running
#[derive(Debug)]
struct ActiveStep {
    text: String,
    level: u8,
    log_type: u64,
    started_at: SystemTime,
    parent: Option<EntryId>,
    messages: Vec<Message>,
}

impl ActiveStep {
    fn new(start: StartEntry, timestamp: SystemTime) -> Self {
        Self {
            text: start.text,
            level: start.level,
            log_type: start.log_type,
            parent: EntryId::new(start.parent),
            started_at: timestamp,
            messages: Vec::new(),
        }
    }

    fn complete(self, id: EntryId, completed_at: SystemTime) -> FinishedStep {
        FinishedStep {
            id,
            started_at: self.started_at,
            text: self.text,
            level: self.level,
            log_type: self.log_type,
            parent: self.parent,
            messages: self.messages,
            completed_at,
        }
    }
}

/// Represents a step that has completed
#[derive(Debug)]
struct FinishedStep {
    id: EntryId,
    text: String,
    level: u8,
    log_type: u64,
    started_at: SystemTime,
    parent: Option<EntryId>,
    messages: Vec<Message>,
    completed_at: SystemTime,
}

#[derive(Serialize, Debug)]
pub struct Summary {
    total_steps: usize,
    started_at: SystemTime,
    completed_at: SystemTime,
    timeline: Vec<TimelineStep>,
}

impl Summary {
    pub fn total_steps(&self) -> usize {
        self.total_steps
    }

    pub fn duration(&self) -> Duration {
        self.started_at
            .duration_since(self.completed_at)
            .unwrap_or_default()
    }
    pub fn timeline(&self) -> &[TimelineStep] {
        &self.timeline
    }
}

/// A step in the build timeline
#[derive(Serialize, Debug, Clone)]
pub struct TimelineStep {
    text: String,
    duration: Duration,
    children: Vec<TimelineStep>,
}

impl TimelineStep {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn children(&self) -> &[TimelineStep] {
        &self.children
    }
}

/// Build hierarchical timeline from finished steps
fn build_timeline(
    finished_steps: &[FinishedStep],
    steps_by_id: &HashMap<EntryId, &FinishedStep>,
) -> Vec<TimelineStep> {
    // Find root steps (level 3 with no parent or parent not in map)
    let root_steps: Vec<&FinishedStep> = finished_steps
        .iter()
        .filter(|step| {
            step.level == 3
                && step
                    .parent
                    .map_or(true, |pid| !steps_by_id.contains_key(&pid))
        })
        .collect();

    root_steps
        .into_iter()
        .map(|step| build_step_tree(step, finished_steps, steps_by_id))
        .collect()
}

/// Recursively build a step tree
fn build_step_tree(
    step: &FinishedStep,
    all_steps: &[FinishedStep],
    steps_by_id: &HashMap<EntryId, &FinishedStep>,
) -> TimelineStep {
    let duration = step
        .completed_at
        .duration_since(step.started_at)
        .unwrap_or_default();

    //TODO: better formatting of durations

    // Find children (level 3 steps whose parent is this step's ID)
    let children: Vec<TimelineStep> = all_steps
        .iter()
        .filter(|child| child.level == 3 && child.parent == Some(step.id))
        .map(|child| build_step_tree(child, all_steps, steps_by_id))
        .collect();

    TimelineStep {
        text: step.text.clone(),
        duration,
        children,
    }
}

/// State machine for parsing nix logs
#[derive(Debug, Default)]
pub struct State {
    /// All parsed steps that have finished. They are sorted in order they finished
    /// TODO: do we need it? could we keep only the btreemap and then create the summary from this?
    finished_steps: Vec<FinishedStep>,
    /// Currently active steps. We keep them in order because we need to get the most recent step started to set any message we could have.
    /// The id seem to be monotonically increasing
    active_steps: BTreeMap<EntryId, ActiveStep>,
}

impl Parser for State {
    type Output = Summary;

    fn handle_start(&mut self, start: StartEntry, timestamp: SystemTime) {
        let Some(entry_id) = EntryId::new(start.id) else {
            warn!(?start.id, "Start entry has zero id, ignoring");
            return;
        };
        let active_step = ActiveStep::new(start, timestamp);
        if let Some(value) = self.active_steps.insert(entry_id, active_step) {
            warn!(?value, "Duplicate start entry id detected");
        };
    }

    fn handle_stop(&mut self, stop: StopEntry, timestamp: SystemTime) {
        let Some(entry_id) = EntryId::new(stop.id) else {
            warn!(?stop.id, "stop entry has zero id, ignoring");
            return;
        };
        if let Some(active_step) = self.active_steps.remove(&entry_id) {
            let finished_step = active_step.complete(entry_id, timestamp);
            self.finished_steps.push(finished_step);
        } else {
            warn!(?stop.id, "Stop entry without matching start");
        }
    }

    fn handle_msg(&mut self, msg: MsgEntry, timestamp: SystemTime) {
        if let Some(mut step) = self.active_steps.last_entry() {
            let message = Message {
                entry: msg,
                timestamp,
            };
            step.get_mut().messages.push(message);
        }
    }

    fn into_output(self, started_at: SystemTime, completed_at: SystemTime) -> Self::Output {
        // Build a map of finished steps by ID for easy lookup
        let steps_by_id: HashMap<EntryId, &FinishedStep> = self
            .finished_steps
            .iter()
            .map(|step| (step.id, step))
            .collect();

        // Build timeline tree (only level 3 steps)
        let timeline = build_timeline(&self.finished_steps, &steps_by_id);

        Summary {
            total_steps: self.finished_steps.len(),
            started_at,
            completed_at,
            timeline,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::SystemTime;

    use tokio::io::AsyncWriteExt;
    use tokio::{fs::File, io::BufReader};

    use crate::nix::logs::{Parser, State};

    #[tokio::test]
    async fn test_parent_child_and_message_ownership() {
        let log_data = r#"@nix {"action":"start","id":45075681771521,"level":3,"parent":0,"text":"checking flake output 'packages'","type":0}
@nix {"action":"start","id":45075681771522,"level":3,"parent":0,"text":"checking derivation packages.x86_64-linux.default","type":0}
@nix {"action":"start","id":45075681771623,"level":5,"parent":0,"text":"copying '/nix/store/lc5bxq4vsgpjjc7i9phdm8s0bxjz0drm-source/pkgs/tools/text/gnupatch/CVE-2019-13638-and-CVE-2018-20969.patch' to the store","type":0}
@nix {"action":"stop","id":45075681771623}
@nix {"action":"msg","level":3,"msg":"derivation evaluated to /nix/store/1bxizw9ww8ibvy4y6c2nflzyh15w2h5w-test-app.drv"}
@nix {"action":"stop","id":45075681771522}
@nix {"action":"start","id":45075681771624,"level":3,"parent":0,"text":"checking derivation packages.x86_64-linux.helper","type":0}
@nix {"action":"msg","level":3,"msg":"derivation evaluated to /nix/store/6m7z2n2f1nmlfpvxqkg1qqws1939v7gc-test-helper.drv"}
@nix {"action":"stop","id":45075681771624}
@nix {"action":"stop","id":45075681771521}"#;

        let cursor = Cursor::new(log_data.as_bytes());
        let reader = BufReader::new(cursor);

        let mut state = State::default();
        state.parse_lines(reader).await;

        // Verify all steps are finished
        assert_eq!(state.active_steps.len(), 0, "All steps should be finished");
        assert_eq!(
            state.finished_steps.len(),
            4,
            "Should have 4 finished steps"
        );

        // Check step 0 (first to finish: 45075681771623 - the copying step)
        let step_0 = &state.finished_steps[0];
        assert_eq!(step_0.level, 5);
        assert_eq!(
            step_0.text,
            "copying '/nix/store/lc5bxq4vsgpjjc7i9phdm8s0bxjz0drm-source/pkgs/tools/text/gnupatch/CVE-2019-13638-and-CVE-2018-20969.patch' to the store"
        );
        assert_eq!(
            step_0.messages.len(),
            0,
            "Step 0 (copying) should have no messages"
        );

        // Check step 1 (second to finish: 45075681771522 - checking derivation default)
        let step_1 = &state.finished_steps[1];
        assert_eq!(
            step_1.text,
            "checking derivation packages.x86_64-linux.default"
        );
        assert_eq!(step_1.level, 3);
        assert_eq!(
            step_1.messages.len(),
            1,
            "Step 1 should have 1 message (the derivation evaluated message)"
        );
        assert_eq!(
            step_1.messages[0].entry.msg,
            "derivation evaluated to /nix/store/1bxizw9ww8ibvy4y6c2nflzyh15w2h5w-test-app.drv"
        );
        assert_eq!(step_1.messages[0].entry.level, 3);

        // Check step 2 (third to finish: 45075681771624 - checking derivation helper)
        let step_2 = &state.finished_steps[2];
        assert_eq!(
            step_2.text,
            "checking derivation packages.x86_64-linux.helper"
        );
        assert_eq!(step_2.level, 3);
        assert_eq!(step_2.messages.len(), 1, "Step 2 should have 1 message");
        assert_eq!(
            step_2.messages[0].entry.msg,
            "derivation evaluated to /nix/store/6m7z2n2f1nmlfpvxqkg1qqws1939v7gc-test-helper.drv"
        );
        assert_eq!(step_2.messages[0].entry.level, 3);

        // Check step 3 (last to finish: 45075681771521 - checking flake output)
        let step_3 = &state.finished_steps[3];
        assert_eq!(step_3.text, "checking flake output 'packages'");
        assert_eq!(step_3.level, 3);
        assert_eq!(step_3.messages.len(), 0, "Step 3 should have no messages");

        println!("\n=== Test Results ===");
        for (i, step) in state.finished_steps.iter().enumerate() {
            println!(
                "Step {} (level {}): {} - {} messages",
                i,
                step.level,
                step.text,
                step.messages.len()
            );
            for (j, msg) in step.messages.iter().enumerate() {
                println!("  Message {}: {}", j + 1, msg.entry.msg);
            }
        }
    }

    #[tokio::test]
    async fn test_run_checks_detailed() {
        let started_at = SystemTime::now();
        let mut handler = State::default();

        let stderr = File::open("test-flake/nix-log").await.unwrap();

        let reader = BufReader::new(stderr);

        handler.parse_lines(reader).await;

        let result = handler.into_output(started_at, SystemTime::now());

        File::create("test-flake/detailed-log2.json")
            .await
            .unwrap()
            .write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
            .await
            .unwrap();
    }
}
