//! Nix log parsing for CI/CD pipelines
//!
//! This module provides a flexible trait-based system for parsing Nix build logs
//! in the internal JSON format. It supports two implementation strategies:
//!
//! ## Implementations
//!
//! ### `LogParsingState` (Simplified)
//! - Tracks only important operations
//! - Produces a compact `BuildSummary`
//! - Use when you need basic timing and package information
//! - Access via: `run_checks_with_logs()`
//!
//! ### `ParsedNixLog` (Detailed)
//! - Tracks all activities with full hierarchy
//! - Preserves parent-child relationships between activities
//! - Includes raw logs for debugging
//! - Categorizes warnings and errors
//! - Use when you need GitHub Actions-like detailed logging
//! - Access via: `run_checks_with_detailed_logs()`
//!
//! ## Usage Examples
//!
//! ```rust,no_run
//! # use procurator_repo_outils::nix::checks::*;
//! # async fn example() -> Result<(), ChecksError> {
//! // Simple usage (backward compatible)
//! let summary = run_checks_with_logs("path/to/flake").await?;
//! println!("Total duration: {:?}", summary.total_duration());
//!
//! // Detailed usage (new)
//! let detailed = run_checks_with_detailed_logs("path/to/flake").await?;
//! for root_id in detailed.root_activities() {
//!     println!("{}", detailed.get_activity_tree(*root_id, 0));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Handlers
//!
//! You can implement the `NixLogHandler` trait to create your own logging strategy:
//!
//! ```rust,no_run
//! # use procurator_repo_outils::nix::checks::*;
//! # use std::time::SystemTime;
//! # use serde::Serialize;
//! #[derive(Default)]
//! struct MyCustomHandler {
//!     // your fields
//! }
//!
//! #[derive(Serialize)]
//! struct MyOutput {
//!     // your output
//! }
//!
//! impl NixLogHandler for MyCustomHandler {
//!     type Output = MyOutput;
//!
//!     fn handle_start(&mut self, start: StartEntry, timestamp: SystemTime) {
//!         // your implementation
//!     }
//!
//!     fn handle_stop(&mut self, stop: StopEntry, timestamp: SystemTime) {
//!         // your implementation
//!     }
//!
//!     fn handle_msg(&mut self, msg: MsgEntry, timestamp: SystemTime) {
//!         // your implementation
//!     }
//!
//!     fn into_output(self, started_at: SystemTime, completed_at: SystemTime) -> Self::Output {
//!         // your finalization
//! #       MyOutput {}
//!     }
//! }
//!
//! # async fn example() -> Result<(), ChecksError> {
//! // Use your custom handler
//! let result = run_checks_generic::<MyCustomHandler>("path/to/flake").await?;
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::ops::Not;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug)]
pub enum ChecksError {
    IoError(std::io::Error),
    ProcessFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    JsonParseError(serde_json::Error),
    InvalidFlakePath(String),
    Timeout(Duration),
}

impl fmt::Display for ChecksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChecksError::IoError(err) => write!(f, "IO error: {}", err),
            ChecksError::ProcessFailed { exit_code, stderr } => {
                write!(
                    f,
                    "Nix process failed with exit code {:?}: {}",
                    exit_code, stderr
                )
            }
            ChecksError::JsonParseError(err) => write!(f, "JSON parse error: {}", err),
            ChecksError::InvalidFlakePath(path) => write!(f, "Invalid flake path: {}", path),
            ChecksError::Timeout(duration) => write!(f, "Process timed out after {:?}", duration),
        }
    }
}

impl std::error::Error for ChecksError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ChecksError::IoError(err) => Some(err),
            ChecksError::JsonParseError(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ChecksError {
    fn from(err: std::io::Error) -> Self {
        ChecksError::IoError(err)
    }
}

impl From<serde_json::Error> for ChecksError {
    fn from(err: serde_json::Error) -> Self {
        ChecksError::JsonParseError(err)
    }
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Copy, Clone)]
struct EntryId(u64);

#[derive(Debug, Serialize, Deserialize)]
pub struct StartEntry {
    id: EntryId,
    level: u8,
    parent: EntryId,
    text: String,
    #[serde(rename = "type")]
    log_type: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StopEntry {
    id: EntryId,
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
    id: EntryId,
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
pub trait NixLogHandler: Default + Send {
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

    /// Finalize and produce output
    fn into_output(self, started_at: SystemTime, completed_at: SystemTime) -> Self::Output;
}

/// Represents a tracked activity with timing information
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
     id: EntryId,
     level: u8,
     parent: EntryId,
     text: String,
     entry_type: u16,
     started_at: SystemTime,
     duration: Option<Duration>,
     children: Vec<EntryId>,
     messages: Vec<String>,
}

/// Complete parsed log with activities and their relationships
#[derive(Debug, Default, Serialize)]
pub struct ParsedNixLog {
     activities: HashMap<EntryId, Activity>,
     root_activities: Vec<EntryId>,
     warnings: Vec<String>,
     errors: Vec<String>,
     raw_logs: Vec<String>,
}

impl ParsedNixLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a tree representation of activities
    pub fn get_activity_tree(&self, activity_id: EntryId, indent: usize) -> String {
        if let Some(activity) = self.activities.get(&activity_id) {
            let mut output = String::new();
            let indent_str = "  ".repeat(indent);

            let duration_str = match activity.duration {
                Some(d) => format!(" ({}ms)", d.as_millis()),
                None => String::from(" (running...)"),
            };

            output.push_str(&format!(
                "{}[{}] {}{}\n",
                indent_str, activity.level, activity.text, duration_str
            ));

            for msg in &activity.messages {
                output.push_str(&format!("{}  └─ {}\n", indent_str, msg));
            }

            for child_id in &activity.children {
                output.push_str(&self.get_activity_tree(*child_id, indent + 1));
            }

            output
        } else {
            String::new()
        }
    }

    /// Get total duration for a specific activity type
    pub fn get_total_duration_by_text(&self, text_contains: &str) -> Duration {
        self.activities
            .values()
            .filter(|a| a.text.contains(text_contains))
            .filter_map(|a| a.duration)
            .sum()
    }

    pub fn root_activities(&self) -> &[EntryId] {
        &self.root_activities
    }

    pub fn raw_logs(&self) -> &[String] {
        &self.raw_logs
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

/// Implement NixLogHandler for ParsedNixLog (new detailed implementation)
impl NixLogHandler for ParsedNixLog {
    type Output = Self;

    fn handle_start(&mut self, start: StartEntry, timestamp: SystemTime) {
        let activity = Activity {
            id: start.id,
            level: start.level,
            parent: start.parent,
            text: start.text,
            entry_type: start.log_type as u16,
            started_at: timestamp,
            duration: None,
            children: Vec::new(),
            messages: Vec::new(),
        };

        // Track root activities (parent == 0)
        if start.parent == EntryId(0) {
            self.root_activities.push(start.id);
        } else if let Some(parent_activity) = self.activities.get_mut(&start.parent) {
            parent_activity.children.push(start.id);
        }

        self.activities.insert(start.id, activity);
    }

    fn handle_stop(&mut self, stop: StopEntry, timestamp: SystemTime) {
        if let Some(activity) = self.activities.get_mut(&stop.id) {
            activity.duration = Some(
                timestamp
                    .duration_since(activity.started_at)
                    .unwrap_or_default()
            );
        }
    }

    fn handle_msg(&mut self, msg: MsgEntry, _timestamp: SystemTime) {
        // Categorize messages
        if msg.level == 1 && msg.msg.contains("warning") {
            self.warnings.push(msg.msg.clone());
        } else if msg.level == 0 || msg.msg.contains("error") {
            self.errors.push(msg.msg.clone());
        }

        // Could attach to most recent activity, or handle differently
        if let Some(last_activity) = self.activities.values_mut().last() {
            last_activity.messages.push(msg.msg);
        }
    }

    fn into_output(self, _started_at: SystemTime, _completed_at: SystemTime) -> Self::Output {
        self
    }
}


// struct Entries {
//     started_at: SystemTime,
//     entry: StartEntry,
//     entries: Vec<>
// }

#[derive(Debug, Default)]
struct LogParsingState {
    active_operations: HashMap<EntryId, (StartEntry, SystemTime)>,
    completed_steps: Vec<StepTiming>,
    important_messages: Vec<String>,
    packages_checked: Vec<String>,
    checks_run: Vec<String>,
}

impl LogParsingState {
    fn into_summary(mut self, started_at: SystemTime, completed_at: SystemTime) -> BuildSummary {
        // Sort steps by start time
        self.completed_steps.sort_by_key(|step| step.started_at);

        BuildSummary {
            started_at,
            completed_at,
            steps: self.completed_steps,
            important_messages: self.important_messages,
            packages_checked: self.packages_checked,
            checks_run: self.checks_run,
        }
    }
}

/// Implement NixLogHandler for LogParsingState (old simplified implementation)
impl NixLogHandler for LogParsingState {
    type Output = BuildSummary;

    fn handle_start(&mut self, start: StartEntry, timestamp: SystemTime) {
        // Extract package/check info
        extract_package_info(
            &start.text,
            &mut self.packages_checked,
            &mut self.checks_run,
        );

        // Only track important operations
        if is_important_operation(&start) {
            info!("Started: {} (level {})", start.text, start.level);
            self.active_operations.insert(start.id, (start, timestamp));
        }
    }

    fn handle_stop(&mut self, stop: StopEntry, timestamp: SystemTime) {
        if let Some((start_entry, start_time)) = self.active_operations.remove(&stop.id) {
            info!(step = &start_entry.text, "Completed");

            self.completed_steps.push(StepTiming {
                id: stop.id,
                name: start_entry.text,
                started_at: start_time,
                completed_at: timestamp,
                level: start_entry.level,
            });
        }
    }

    fn handle_msg(&mut self, msg: MsgEntry, _timestamp: SystemTime) {
        if is_important_message(&msg.msg) {
            self.important_messages.push(strip_ansi_codes(&msg.msg));
        }
    }

    fn into_output(self, started_at: SystemTime, completed_at: SystemTime) -> Self::Output {
        self.into_summary(started_at, completed_at)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StepTiming {
    id: EntryId,
    name: String,
    started_at: SystemTime,
    completed_at: SystemTime,
    level: u8,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BuildSummary {
    started_at: SystemTime,
    completed_at: SystemTime,
    steps: Vec<StepTiming>,
    important_messages: Vec<String>,
    packages_checked: Vec<String>,
    checks_run: Vec<String>,
}

impl BuildSummary {
    pub fn new(steps: Vec<StepTiming>) -> Self {
        let now = SystemTime::now();
        Self {
            started_at: now,
            completed_at: now,
            steps,
            important_messages: Vec::new(),
            packages_checked: Vec::new(),
            checks_run: Vec::new(),
        }
    }

    pub fn total_duration(&self) -> Duration {
        self.completed_at
            .duration_since(self.started_at)
            .expect("we should be able to get the time")
    }

    pub fn steps(&self) -> &Vec<StepTiming> {
        &self.steps
    }

    pub fn important_messages(&self) -> &Vec<String> {
        &self.important_messages
    }

    pub fn packages_checked(&self) -> &Vec<String> {
        &self.packages_checked
    }

    pub fn checks_run(&self) -> &Vec<String> {
        &self.checks_run
    }

    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub fn completed_at(&self) -> SystemTime {
        self.completed_at
    }
}

/// Generic function to parse Nix logs using any handler implementation
async fn parse_lines_generic<H: NixLogHandler>(
    handler: &mut H,
    buffer: impl AsyncRead + Unpin,
) {
    let reader = BufReader::new(buffer);
    let mut lines = reader.lines();

    while let Ok(Some(raw_line)) = lines.next_line().await {
        if let Some(json_part) = raw_line.strip_prefix("@nix ") {
            let timestamp = SystemTime::now();

            match serde_json::from_str::<LogEntry>(json_part) {
                Ok(entry) => handler.process_entry(entry, timestamp),
                Err(err) => {
                    warn!(%raw_line, error=%err, "Failed to parse nix log entry");
                }
            }
        }
    }
}

/// Generic function to run checks with any log handler
pub async fn run_checks_generic<H: NixLogHandler>(
    flake_path: &str,
) -> Result<H::Output, ChecksError> {
    // Validate flake path
    if flake_path.is_empty() {
        return Err(ChecksError::InvalidFlakePath(
            "Flake path cannot be empty".to_string(),
        ));
    }

    let started_at = SystemTime::now();
    let mut handler = H::default();

    let mut child = Command::new("nix")
        .arg("flake")
        .arg("check")
        .arg(flake_path)
        .arg("--print-build-logs")
        .arg("--log-format")
        .arg("internal-json")
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stderr = child.stderr.take().unwrap();

    parse_lines_generic(&mut handler, stderr).await;

    // Wait for the process to finish
    let status = child.wait().await?;

    if status.success().not() {
        return Err(ChecksError::ProcessFailed {
            exit_code: status.code(),
            stderr: "Build failed".to_string(),
        });
    }

    Ok(handler.into_output(started_at, SystemTime::now()))
}

/// Backward compatible function using the old simplified handler
pub async fn run_checks_with_logs(flake_path: &str) -> Result<BuildSummary, ChecksError> {
    run_checks_generic::<LogParsingState>(flake_path).await
}

/// New function using the detailed handler
pub async fn run_checks_with_detailed_logs(
    flake_path: &str,
) -> Result<ParsedNixLog, ChecksError> {
    run_checks_generic::<ParsedNixLog>(flake_path).await
}

fn is_important_operation(start: &StartEntry) -> bool {
    // Only track level 3 operations and key level 6 operations
    let is_level_important = start.level <= 3;

    // Also track specific important level 6 operations
    let is_specific_level6 =
        start.level == 6 && start.text.contains("querying info about missing paths");

    is_level_important || is_specific_level6
}

fn is_important_message(msg: &str) -> bool {
    msg.contains("warning")
        || msg.contains("error")
        || msg.contains("derivation evaluated to")
        || msg.contains("incompatible systems")
        || msg.contains("Use '--all-systems'")
}

fn extract_package_info(text: &str, packages: &mut Vec<String>, checks: &mut Vec<String>) {
    match text {
        s if s.starts_with("checking derivation packages.") => {
            if let Some(pkg_name) = s
                .strip_prefix("checking derivation packages.")
                .and_then(|pkg| pkg.split('.').last())
            {
                let name = pkg_name.to_string();
                if !packages.contains(&name) {
                    packages.push(name);
                }
            }
        }
        s if s.starts_with("checking derivation checks.") => {
            if let Some(check_name) = s
                .strip_prefix("checking derivation checks.")
                .and_then(|check| check.split('.').last())
            {
                let name = check_name.to_string();
                if !checks.contains(&name) {
                    checks.push(name);
                }
            }
        }
        _ => {} // Do nothing for other cases
    }
}

fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // Skip '['
            while let Some(ch) = chars.next() {
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use tokio::{fs::File, io::AsyncWriteExt};

    use super::*;

    //NOTE: this test is slow
    #[tokio::test]
    async fn test_run_checks() {
        let mut flake_path: String = env!("CARGO_MANIFEST_DIR").into();
        flake_path.push('/');
        flake_path.push_str("test-flake");

        let result = run_checks_with_logs(&flake_path).await.unwrap();

        assert_eq!(result.total_duration().as_millis() > 10, true);

        File::create("test-flake/dummy-file2.json")
            .await
            .unwrap()
            .write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_run_checks_detailed() {
        let mut flake_path: String = env!("CARGO_MANIFEST_DIR").into();
        flake_path.push('/');
        flake_path.push_str("test-flake");

        let result = run_checks_with_detailed_logs(&flake_path).await.unwrap();

        // Verify we captured activities
        assert!(!result.root_activities().is_empty());

        // Get tree representation
        for root_id in result.root_activities() {
            let tree = result.get_activity_tree(*root_id, 0);
            println!("Activity tree:\n{}", tree);
        }

        File::create("test-flake/detailed-log.json")
            .await
            .unwrap()
            .write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
            .await
            .unwrap();
    }

}
