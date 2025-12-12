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
pub enum NixLogEntry {
    Start(StartEntry),
    Stop(StopEntry),
    Msg(MsgEntry),
    Result(ResultEntry),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StepTiming {
    id: u64,
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

#[derive(Debug, Default)]
struct LogParsingState {
    active_operations: HashMap<u64, (StartEntry, SystemTime)>,
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

fn process_entry(state: &mut LogParsingState, entry: NixLogEntry, timestamp: SystemTime) {
    match entry {
        NixLogEntry::Start(start) => {
            handle_start_entry(state, start, timestamp);
        }
        NixLogEntry::Stop(stop) => {
            handle_stop_entry(state, stop, timestamp);
        }
        NixLogEntry::Msg(msg) => {
            handle_msg_entry(state, msg);
        }
        NixLogEntry::Result(_) => {
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

            match serde_json::from_str::<NixLogEntry>(json_part) {
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
    use super::*;
    use std::io::Cursor;

    // Helper function to create test log data
    fn create_test_log_data() -> &'static str {
        r#"@nix {"action":"start","id":1,"level":3,"parent":0,"text":"evaluating flake","type":0}
@nix {"action":"start","id":2,"level":3,"parent":0,"text":"checking flake output 'packages'","type":0}
@nix {"action":"start","id":3,"level":3,"parent":0,"text":"checking derivation packages.x86_64-linux.default","type":0}
@nix {"action":"start","id":4,"level":5,"parent":0,"text":"copying '/nix/store/abc-source/file.sh' to the store","type":0}
@nix {"action":"stop","id":4}
@nix {"action":"msg","level":3,"msg":"derivation evaluated to /nix/store/bh2mjj5fqdcy3ss17shhwkki96s2cvgw-dummy-0.1.0.drv"}
@nix {"action":"stop","id":3}
@nix {"action":"start","id":5,"level":3,"parent":0,"text":"checking derivation packages.x86_64-linux.state","type":0}
@nix {"action":"msg","level":3,"msg":"derivation evaluated to /nix/store/p2zrfafw065x04p4926rsciblmz3py42-state-lock.drv"}
@nix {"action":"stop","id":5}
@nix {"action":"stop","id":2}
@nix {"action":"start","id":6,"level":3,"parent":0,"text":"checking flake output 'checks'","type":0}
@nix {"action":"start","id":7,"level":3,"parent":0,"text":"checking derivation checks.x86_64-linux.dummy-test","type":0}
@nix {"action":"msg","level":3,"msg":"derivation evaluated to /nix/store/b57yk9hqikmf8194anazx6hw2sinj0jb-dummy-test.drv"}
@nix {"action":"stop","id":7}
@nix {"action":"start","id":8,"level":3,"parent":0,"text":"checking derivation checks.x86_64-linux.licenses","type":0}
@nix {"action":"stop","id":8}
@nix {"action":"start","id":9,"level":3,"parent":0,"text":"checking derivation checks.x86_64-linux.formatting","type":0}
@nix {"action":"stop","id":9}
@nix {"action":"stop","id":6}
@nix {"action":"start","id":10,"level":3,"parent":0,"text":"running 5 flake checks","type":0}
@nix {"action":"start","id":11,"level":6,"parent":0,"text":"querying info about missing paths","type":0}
@nix {"action":"stop","id":11}
@nix {"action":"stop","id":10}
@nix {"action":"stop","id":1}
@nix {"action":"msg","level":1,"msg":"\u001b[35;1mwarning:\u001b[0m The check omitted these incompatible systems: aarch64-darwin, aarch64-linux, x86_64-darwin\nUse '--all-systems' to check all."}"#
    }

    fn create_minimal_success_log() -> &'static str {
        r#"@nix {"action":"start","id":1,"level":3,"parent":0,"text":"evaluating flake","type":0}
@nix {"action":"stop","id":1}"#
    }

    fn create_error_log() -> &'static str {
        r#"@nix {"action":"start","id":1,"level":3,"parent":0,"text":"evaluating flake","type":0}
@nix {"action":"msg","level":1,"msg":"error: flake evaluation failed"}
@nix {"action":"stop","id":1}"#
    }

    #[test]
    fn test_extract_package_info() {
        let mut packages = Vec::new();
        let mut checks = Vec::new();

        // Test package extraction
        extract_package_info(
            "checking derivation packages.x86_64-linux.default",
            &mut packages,
            &mut checks,
        );
        assert_eq!(packages, vec!["default"]);
        assert!(checks.is_empty());

        // Test check extraction
        extract_package_info(
            "checking derivation checks.x86_64-linux.dummy-test",
            &mut packages,
            &mut checks,
        );
        assert_eq!(packages, vec!["default"]);
        assert_eq!(checks, vec!["dummy-test"]);

        // Test state package
        extract_package_info(
            "checking derivation packages.x86_64-linux.state",
            &mut packages,
            &mut checks,
        );
        assert_eq!(packages, vec!["default", "state"]);

        // Test more checks
        extract_package_info(
            "checking derivation checks.x86_64-linux.licenses",
            &mut packages,
            &mut checks,
        );
        extract_package_info(
            "checking derivation checks.x86_64-linux.formatting",
            &mut packages,
            &mut checks,
        );
        assert_eq!(checks, vec!["dummy-test", "licenses", "formatting"]);

        // Test no duplicates
        extract_package_info(
            "checking derivation packages.x86_64-linux.default",
            &mut packages,
            &mut checks,
        );
        assert_eq!(packages, vec!["default", "state"]); // Still only two

        // Test unrelated text
        extract_package_info("some other text", &mut packages, &mut checks);
        assert_eq!(packages.len(), 2);
        assert_eq!(checks.len(), 3);
    }

    #[test]
    fn test_is_important_operation() {
        // Level 3 operations should be important
        let important_level3 = StartEntry {
            id: 1,
            level: 3,
            parent: 0,
            text: "evaluating flake".to_string(),
            log_type: 0,
        };
        assert!(is_important_operation(&important_level3));

        // Level 5 operations should not be important
        let unimportant_level5 = StartEntry {
            id: 2,
            level: 5,
            parent: 0,
            text: "copying file to store".to_string(),
            log_type: 0,
        };
        assert!(!is_important_operation(&unimportant_level5));

        // Specific level 6 operation should be important
        let important_level6 = StartEntry {
            id: 3,
            level: 6,
            parent: 0,
            text: "querying info about missing paths".to_string(),
            log_type: 0,
        };
        assert!(is_important_operation(&important_level6));

        // Other level 6 operations should not be important
        let unimportant_level6 = StartEntry {
            id: 4,
            level: 6,
            parent: 0,
            text: "some other level 6 operation".to_string(),
            log_type: 0,
        };
        assert!(!is_important_operation(&unimportant_level6));
    }

    #[test]
    fn test_is_important_message() {
        assert!(is_important_message("warning: something happened"));
        assert!(is_important_message("error: build failed"));
        assert!(is_important_message(
            "derivation evaluated to /nix/store/..."
        ));
        assert!(is_important_message("incompatible systems detected"));
        assert!(is_important_message("Use '--all-systems' to check all"));

        assert!(!is_important_message("normal log message"));
        assert!(!is_important_message("copying file"));
        assert!(!is_important_message(""));
    }

    #[test]
    fn test_strip_ansi_codes() {
        let input =
            "\u{001b}[35;1mwarning:\u{001b}[0m The check omitted these incompatible systems";
        let expected = "warning: The check omitted these incompatible systems";
        assert_eq!(strip_ansi_codes(input), expected);

        // Test with no ANSI codes
        let plain = "plain text";
        assert_eq!(strip_ansi_codes(plain), "plain text");

        // Test with multiple ANSI sequences - FIXED the typos
        let complex = "\u{001b}[31mred\u{001b}[0m and \u{001b}[32mgreen\u{001b}[0m";
        assert_eq!(strip_ansi_codes(complex), "red and green");
    }

    #[tokio::test]
    async fn test_parse_lines_with_real_data() {
        let mut state = LogParsingState::default();
        let cursor = Cursor::new(create_test_log_data().as_bytes());

        parse_lines(&mut state, cursor).await;

        // Should have captured important steps
        assert!(!state.completed_steps.is_empty());

        // Should have found packages
        assert_eq!(state.packages_checked, vec!["default", "state"]);

        // Should have found checks
        assert_eq!(
            state.checks_run,
            vec!["dummy-test", "licenses", "formatting"]
        );

        // Should have important messages
        assert!(!state.important_messages.is_empty());
        assert!(state
            .important_messages
            .iter()
            .any(|msg| msg.contains("derivation evaluated")));
        assert!(state
            .important_messages
            .iter()
            .any(|msg| msg.contains("warning:")));

        // Verify specific steps were captured
        let step_names: Vec<&String> = state.completed_steps.iter().map(|s| &s.name).collect();
        assert!(step_names.contains(&&"evaluating flake".to_string()));
        assert!(step_names.contains(&&"checking flake output 'packages'".to_string()));
        assert!(step_names.contains(&&"running 5 flake checks".to_string()));

        // Should not have captured level 5 operations
        assert!(!step_names.iter().any(|name| name.contains("copying")));
    }

    #[tokio::test]
    async fn test_parse_lines_minimal_success() {
        let mut state = LogParsingState::default();
        let cursor = Cursor::new(create_minimal_success_log().as_bytes());

        parse_lines(&mut state, cursor).await;

        assert_eq!(state.completed_steps.len(), 1);
        assert_eq!(state.completed_steps[0].name, "evaluating flake");
        assert_eq!(state.completed_steps[0].level, 3);
    }

    #[test]
    fn test_log_parsing_state_into_summary() {
        let mut state = LogParsingState::default();

        // Add some test data
        state.packages_checked.push("test-package".to_string());
        state.checks_run.push("test-check".to_string());
        state.important_messages.push("Test message".to_string());

        let start_time = SystemTime::now();
        let end_time = start_time + Duration::from_millis(100);

        state.completed_steps.push(StepTiming {
            id: 1,
            name: "test step".to_string(),
            started_at: start_time,
            completed_at: end_time,
            level: 3,
        });

        let summary = state.into_summary(start_time, end_time);

        assert_eq!(summary.started_at, start_time);
        assert_eq!(summary.completed_at, end_time);
        assert_eq!(summary.total_duration(), Duration::from_millis(100));
        assert_eq!(summary.steps.len(), 1);
        assert_eq!(summary.packages_checked, vec!["test-package"]);
        assert_eq!(summary.checks_run, vec!["test-check"]);
        assert_eq!(summary.important_messages, vec!["Test message"]);
    }

    #[tokio::test]
    async fn test_parse_lines_with_errors() {
        let mut state = LogParsingState::default();
        let cursor = Cursor::new(create_error_log().as_bytes());

        parse_lines(&mut state, cursor).await;

        assert!(state
            .important_messages
            .iter()
            .any(|msg| msg.contains("error:")));
    }

    #[tokio::test]
    async fn test_parse_lines_with_invalid_json() {
        let mut state = LogParsingState::default();
        let invalid_data = r#"@nix invalid json
@nix {"action":"start","id":1,"level":3,"parent":0,"text":"valid entry","type":0}
@nix {"action":"stop","id":1}
not a nix line"#;

        let cursor = Cursor::new(invalid_data.as_bytes());
        parse_lines(&mut state, cursor).await;

        // Should still process valid entries despite invalid ones
        assert_eq!(state.completed_steps.len(), 1);
        assert_eq!(state.completed_steps[0].name, "valid entry");
    }

    #[test]
    fn test_process_entry_timing() {
        let mut state = LogParsingState::default();
        let start_time = SystemTime::now();

        // Process start entry
        let start_entry = StartEntry {
            id: 123,
            level: 3,
            parent: 0,
            text: "test operation".to_string(),
            log_type: 0,
        };

        process_entry(&mut state, NixLogEntry::Start(start_entry), start_time);

        // Should be in active operations
        assert!(state.active_operations.contains_key(&123));
        assert_eq!(state.completed_steps.len(), 0);

        // Process stop entry
        let end_time = start_time + Duration::from_millis(50);
        let stop_entry = StopEntry { id: 123 };

        process_entry(&mut state, NixLogEntry::Stop(stop_entry), end_time);

        // Should be moved to completed steps
        assert!(!state.active_operations.contains_key(&123));
        assert_eq!(state.completed_steps.len(), 1);

        let step = &state.completed_steps[0];
        assert_eq!(step.id, 123);
        assert_eq!(step.name, "test operation");
        assert_eq!(step.started_at, start_time);
        assert_eq!(step.completed_at, end_time);
    }

    #[test]
    fn test_process_entry_unmatched_stop() {
        let mut state = LogParsingState::default();

        // Process stop entry without corresponding start
        let stop_entry = StopEntry { id: 999 };
        process_entry(&mut state, NixLogEntry::Stop(stop_entry), SystemTime::now());

        // Should not crash or create invalid entries
        assert_eq!(state.completed_steps.len(), 0);
        assert!(!state.active_operations.contains_key(&999));
    }

    #[tokio::test]
    async fn test_run_checks_invalid_path() {
        let result = run_checks_with_logs("").await;
        assert!(result.is_err());

        if let Err(ChecksError::InvalidFlakePath(msg)) = result {
            assert_eq!(msg, "Flake path cannot be empty");
        } else {
            panic!("Expected InvalidFlakePath error");
        }
    }

    #[test]
    fn test_build_summary_total_duration() {
        let start = SystemTime::now();
        let end = start + Duration::from_secs(5);

        let summary = BuildSummary {
            started_at: start,
            completed_at: end,
            steps: vec![],
            important_messages: vec![],
            packages_checked: vec![],
            checks_run: vec![],
        };

        assert_eq!(summary.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn test_step_timing_order() {
        let mut state = LogParsingState::default();
        let base_time = SystemTime::now();

        // Add steps in reverse chronological order
        state.completed_steps.push(StepTiming {
            id: 2,
            name: "second".to_string(),
            started_at: base_time + Duration::from_millis(100),
            completed_at: base_time + Duration::from_millis(110),
            level: 3,
        });

        state.completed_steps.push(StepTiming {
            id: 1,
            name: "first".to_string(),
            started_at: base_time,
            completed_at: base_time + Duration::from_millis(10),
            level: 3,
        });

        let summary = state.into_summary(base_time, base_time + Duration::from_millis(200));

        // Should be sorted by start time
        assert_eq!(summary.steps[0].name, "first");
        assert_eq!(summary.steps[1].name, "second");
    }

    #[tokio::test]
    async fn test_flake_not_found() {
        let flake_path = "procurator/test-flake";
        let result = run_checks_with_logs(flake_path).await;

        assert!(result.is_err());

        match result {
            Err(ChecksError::ProcessFailed { exit_code, stderr }) => {
                // Nix should exit with non-zero code when flake is not found
                assert!(exit_code.unwrap_or(0) != 0);
                assert_eq!(stderr, "Build failed");
            }
            Err(other_error) => {
                // Could also be IoError if nix command is not available
                println!("Got different error type: {:?}", other_error);
                // Don't fail test if it's just nix not being installed
                if matches!(other_error, ChecksError::IoError(_)) {
                    println!("Skipping test - nix command not available");
                    return;
                }
                panic!("Expected ProcessFailed error, got: {:?}", other_error);
            }
            Ok(summary) => {
                panic!("Expected error but got success: {:?}", summary);
            }
        }
    }

    //NOTE: this test is slow
    #[tokio::test]
    async fn test_run_checks() {
        let mut flake_path: String = env!("CARGO_MANIFEST_DIR").into();
        flake_path.push('/');
        flake_path.push_str("test-flake");

        let result = run_checks_with_logs(&flake_path).await.unwrap();

        assert_eq!(result.total_duration().as_millis() > 10, true);
    }
}
