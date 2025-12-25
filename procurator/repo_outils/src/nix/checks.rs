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
    parent: u64,
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

struct Entries {
    started_at: SystemTime,
    entry: StartEntry,
    entries: Vec<>
}

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



fn handle_start_entry(state: &mut LogParsingState, start: StartEntry, timestamp: SystemTime) {
    // Extract package/check info
    extract_package_info(
        &start.text,
        &mut state.packages_checked,
        &mut state.checks_run,
    );

    // Only track important operations
    if is_important_operation(&start) {
        info!("Started: {} (level {})", start.text, start.level);
        state.active_operations.insert(start.id, (start, timestamp));
    }
}

fn handle_stop_entry(state: &mut LogParsingState, stop: StopEntry, timestamp: SystemTime) {
    if let Some((start_entry, start_time)) = state.active_operations.remove(&stop.id) {
        info!(step = &start_entry.text, "Completed");

        state.completed_steps.push(StepTiming {
            id: stop.id,
            name: start_entry.text,
            started_at: start_time,
            completed_at: timestamp,
            level: start_entry.level,
        });
    }
}

fn handle_msg_entry(state: &mut LogParsingState, msg: MsgEntry) {
    if is_important_message(&msg.msg) {
        state.important_messages.push(strip_ansi_codes(&msg.msg));
    }
}

fn process_entry(state: &mut LogParsingState, entry: LogEntry, timestamp: SystemTime) {
    match entry {
        LogEntry::Start(start) => {
            handle_start_entry(state, start, timestamp);
        }
        LogEntry::Stop(stop) => {
            handle_stop_entry(state, stop, timestamp);
        }
        LogEntry::Msg(msg) => {
            handle_msg_entry(state, msg);
        }
        LogEntry::Result(_) => {
            // Skip for now
        }
    }
}

async fn parse_lines(state: &mut LogParsingState, buffer: impl AsyncRead + Unpin) {
    let reader = BufReader::new(buffer);
    let mut lines = reader.lines();

    while let Ok(Some(raw_line)) = lines.next_line().await {
        if let Some(json_part) = raw_line.strip_prefix("@nix ") {
            let timestamp = SystemTime::now();

            match serde_json::from_str::<LogEntry>(json_part) {
                Ok(entry) => process_entry(state, entry, timestamp),

                Err(err) => {
                    warn!(%raw_line, error=%err, "Failed to parse nix log entry");
                }
            }
        }
    }
}

pub async fn run_checks_with_logs(flake_path: &str) -> Result<BuildSummary, ChecksError> {
    // Validate flake path
    if flake_path.is_empty() {
        return Err(ChecksError::InvalidFlakePath(
            "Flake path cannot be empty".to_string(),
        ));
    }

    let started_at = SystemTime::now();

    let mut state = LogParsingState::default();

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

    parse_lines(&mut state, stderr).await;

    // Wait for the process to finish
    let status = child.wait().await?;

    if status.success().not() {
        return Err(ChecksError::ProcessFailed {
            exit_code: status.code(),
            stderr: "Build failed".to_string(),
        });
    }

    Ok(state.into_summary(started_at, SystemTime::now()))
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

        File::create("test-flake/dummy-file.json")
            .await
            .unwrap()
            .write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
            .await
            .unwrap();
    }
}
