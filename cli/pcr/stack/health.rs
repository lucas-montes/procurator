use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::stack::config::HealthCheckConfig;

/// Errors produced by healthcheck execution.
#[derive(Debug)]
pub enum HealthCheckError {
    /// The test command could not be spawned.
    Spawn(String),
    /// The test command was terminated because it exceeded the configured timeout.
    Timeout,
    /// The test command exited with a non-zero status.
    Failed(i32),
    /// The healthcheck test configuration has an invalid format.
    ParseCmd,
}

impl std::fmt::Display for HealthCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(msg) => write!(f, "healthcheck spawn error: {msg}"),
            Self::Timeout => write!(f, "healthcheck timed out"),
            Self::Failed(code) => write!(f, "healthcheck failed with exit code {code}"),
            Self::ParseCmd => write!(f, "invalid healthcheck test format"),
        }
    }
}

impl std::error::Error for HealthCheckError {}

// ---------------------------------------------------------------------------
// Test-command parser
// ---------------------------------------------------------------------------

/// Resolve a healthcheck `test` value into a `(program, args)` pair suitable
/// for `tokio::process::Command`.
///
/// Supports three formats matching Docker Compose semantics:
///
/// | Input | Behaviour |
/// |---|---|
/// | `"curl -f http://localhost"` | `sh -c "curl -f http://localhost"` |
/// | `["CMD-SHELL", "pg_isready -U postgres"]` | `sh -c "pg_isready -U postgres"` |
/// | `["CMD", "curl", "-f", "http://localhost"]` | direct exec: `curl -f http://localhost` |
/// | `["curl", "-f", "http://localhost"]` | direct exec (no prefix) |
///
/// NOTE: Related to `parse_cmd()` in service.rs (line 639). Both implement
/// the same string/array dispatch. This version is a superset with
/// `CMD`/`CMD-SHELL` prefix handling. See service.rs for the simpler variant.
fn parse_test(test: &serde_json::Value) -> Result<(String, Vec<String>), HealthCheckError> {
    match test {
        serde_json::Value::String(s) => Ok(("sh".to_string(), vec!["-c".to_string(), s.clone()])),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Err(HealthCheckError::ParseCmd);
            }
            let first = arr[0].as_str().unwrap_or("");
            let rest = &arr[1..];

            if first.eq_ignore_ascii_case("CMD-SHELL") {
                // CMD-SHELL: join rest as a shell command string
                let parts: Vec<&str> = rest.iter().filter_map(|v| v.as_str()).collect();
                if parts.is_empty() {
                    return Err(HealthCheckError::ParseCmd);
                }
                let joined = parts.join(" ");
                Ok(("sh".to_string(), vec!["-c".to_string(), joined]))
            } else if first.eq_ignore_ascii_case("CMD") {
                // CMD: exec the rest directly (skip the CMD prefix)
                let prog = rest
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or(HealthCheckError::ParseCmd)?;
                let args: Vec<String> = rest[1..]
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                Ok((prog.to_string(), args))
            } else {
                // No prefix: direct exec with first element as program
                let prog = first.to_string();
                let args: Vec<String> = arr[1..]
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                Ok((prog, args))
            }
        }
        _ => Err(HealthCheckError::ParseCmd),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run a single healthcheck probe.
///
/// Spawns the test command, waits for it to complete within the configured
/// timeout, and returns `Ok(())` if it exits with status zero.
///
/// # Errors
///
/// Returns [`HealthCheckError`] if the command cannot be spawned, exceeds the
/// timeout, or exits with a non-zero status.
pub async fn run_healthcheck(
    config: &HealthCheckConfig,
    working_dir: &Path,
) -> Result<(), HealthCheckError> {
    let (prog, args) = parse_test(config.test())?;

    let mut cmd = Command::new(&prog);
    cmd.args(&args).current_dir(working_dir).kill_on_drop(true);

    let timeout_dur = Duration::from_secs(config.timeout_secs());

    let child = cmd
        .spawn()
        .map_err(|e| HealthCheckError::Spawn(e.to_string()))?;

    match timeout(timeout_dur, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                Ok(())
            } else {
                Err(HealthCheckError::Failed(output.status.code().unwrap_or(-1)))
            }
        }
        Ok(Err(e)) => Err(HealthCheckError::Spawn(e.to_string())),
        Err(_elapsed) => Err(HealthCheckError::Timeout),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── parse_test ──────────────────────────────────────────────────────

    #[test]
    fn test_parse_string() {
        let val = json!("curl -f http://localhost");
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "sh");
        assert_eq!(args, ["-c", "curl -f http://localhost"]);
    }

    #[test]
    fn test_parse_cmd_shell() {
        let val = json!(["CMD-SHELL", "pg_isready -U postgres"]);
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "sh");
        assert_eq!(args, ["-c", "pg_isready -U postgres"]);
    }

    #[test]
    fn test_parse_cmd_shell_multi_part() {
        let val = json!(["CMD-SHELL", "pg_isready", "-U", "postgres"]);
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "sh");
        assert_eq!(args, ["-c", "pg_isready -U postgres"]);
    }

    #[test]
    fn test_parse_cmd_prefix() {
        let val = json!(["CMD", "curl", "-f", "http://localhost"]);
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "curl");
        assert_eq!(args, ["-f", "http://localhost"]);
    }

    #[test]
    fn test_parse_cmd_prefix_single_arg() {
        let val = json!(["CMD", "true"]);
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "true");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_unprefixed_array() {
        let val = json!(["curl", "-f", "http://localhost"]);
        let (prog, args) = parse_test(&val).unwrap();
        assert_eq!(prog, "curl");
        assert_eq!(args, ["-f", "http://localhost"]);
    }

    #[test]
    fn test_parse_empty_array() {
        let val = json!([]);
        assert!(parse_test(&val).is_err());
    }

    #[test]
    fn test_parse_invalid_type() {
        let val = json!(42);
        assert!(parse_test(&val).is_err());
    }

    #[test]
    fn test_parse_cmd_shell_empty() {
        let val = json!(["CMD-SHELL"]);
        assert!(parse_test(&val).is_err());
    }

    #[test]
    fn test_parse_cmd_empty() {
        let val = json!(["CMD"]);
        assert!(parse_test(&val).is_err());
    }

    #[test]
    fn test_parse_null() {
        let val = json!(null);
        assert!(parse_test(&val).is_err());
    }

    // ── run_healthcheck ─────────────────────────────────────────────────

    fn make_hc(test: serde_json::Value) -> HealthCheckConfig {
        let json = serde_json::json!({
            "test": test,
            "interval_secs": 30,
            "timeout_secs": 10,
            "retries": 3,
        });
        serde_json::from_value(json).unwrap()
    }

    #[tokio::test]
    async fn test_runner_success() {
        let hc = make_hc(json!(["CMD", "true"]));
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runner_failure() {
        let hc = make_hc(json!(["CMD", "false"]));
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HealthCheckError::Failed(1)));
    }

    #[tokio::test]
    async fn test_runner_shell_success() {
        let hc = make_hc(json!("test 1 -eq 1"));
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runner_shell_failure() {
        let hc = make_hc(json!("test 1 -eq 2"));
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_runner_spawn_error() {
        let hc = make_hc(json!(["CMD", "/nonexistent/binary"]));
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(matches!(result.unwrap_err(), HealthCheckError::Spawn(_)));
    }

    // ── Deserialization ─────────────────────────────────────────────────

    #[test]
    fn test_deserialize_valid_healthcheck() {
        let json = serde_json::json!({
            "test": "curl -f http://localhost",
            "interval_secs": 15,
            "timeout_secs": 8,
            "retries": 4,
        });
        let hc: HealthCheckConfig = serde_json::from_value(json).unwrap();
        assert_eq!(hc.interval_secs(), 15);
        assert_eq!(hc.timeout_secs(), 8);
        assert_eq!(hc.retries(), 4);
    }

    #[test]
    fn test_deserialize_defaults() {
        // Minimal config — only `test` is required; the rest should use serde defaults.
        let json = serde_json::json!({
            "test": "echo ok",
        });
        let hc: HealthCheckConfig = serde_json::from_value(json).unwrap();
        assert_eq!(hc.interval_secs(), 30);
        assert_eq!(hc.timeout_secs(), 10);
        assert_eq!(hc.retries(), 3);
    }

    #[test]
    fn test_deserialize_array_test() {
        let json = serde_json::json!({
            "test": ["CMD", "curl", "-f", "http://localhost"],
        });
        let hc: HealthCheckConfig = serde_json::from_value(json).unwrap();
        assert_eq!(hc.interval_secs(), 30); // default
    }

    // ── Timeout ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_runner_timeout() {
        // Run a command that sleeps for 10s with a 1s timeout → should time out.
        let json = serde_json::json!({
            "test": ["CMD", "sleep", "10"],
            "timeout_secs": 1,
        });
        let hc: HealthCheckConfig = serde_json::from_value(json).unwrap();
        let result = run_healthcheck(&hc, Path::new(".")).await;
        assert!(matches!(result.unwrap_err(), HealthCheckError::Timeout));
    }
}
