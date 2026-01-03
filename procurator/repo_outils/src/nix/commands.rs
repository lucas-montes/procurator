use serde::Serialize;
use std::{ops::Not, path::Path, time::SystemTime};
use tokio::{io::BufReader, process::Command};

use super::logs::{Error as LogError, Parser, State, Summary};

/// Errors specific to each command type
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    ProcessFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    JsonParse(serde_json::Error),
    InvalidFlakePath(String),
    LogParsing(LogError),
    BuildOutputMissing,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "IO error: {}", err),
            Error::ProcessFailed { exit_code, stderr } => {
                write!(
                    f,
                    "Process failed with exit code {:?}: {}",
                    exit_code, stderr
                )
            }
            Error::JsonParse(err) => write!(f, "Failed to parse JSON output: {}", err),
            Error::InvalidFlakePath(path) => write!(f, "Invalid flake path: {}", path),
            Error::LogParsing(err) => write!(f, "Log parsing error: {}", err),
            Error::BuildOutputMissing => write!(f, "Build output missing"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            Error::JsonParse(err) => Some(err),
            Error::LogParsing(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::JsonParse(err)
    }
}

impl From<LogError> for Error {
    fn from(err: LogError) -> Self {
        Error::LogParsing(err)
    }
}

type Result<T> = std::result::Result<T, Error>;

async fn run_command<H: Parser>(mut command: Command) -> Result<H::Output> {
    let started_at = SystemTime::now();
    let mut handler = H::default();

    let mut child = command.stderr(std::process::Stdio::piped()).spawn()?;

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    handler.parse_lines(reader).await;

    // Wait for the process to finish
    let status = child.wait().await?;

    if status.success().not() {
        return Err(Error::ProcessFailed {
            exit_code: status.code(),
            stderr: "Build failed".to_string(),
        });
    }

    Ok(handler.into_output(started_at, SystemTime::now()))
}

/// Result from `nix flake check`
#[derive(Debug, Serialize)]
pub struct CheckResult {
    summary: Summary,
}

/// Run `nix flake check` - returns detailed summary and success status
pub async fn flake_check(flake_path: impl AsRef<Path>) -> Result<CheckResult> {
    let path = flake_path.as_ref();
    validate_path(path)?;

    let mut command = Command::new("nix");
    command
        .arg("flake")
        .arg("check")
        .arg(path)
        .arg("--print-build-logs")
        .arg("--log-format")
        .arg("internal-json");

    let summary = run_command::<State>(command).await?;

    Ok(CheckResult { summary })
}

/// Validate that a path is reasonable for a flake
fn validate_path(path: &Path) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| Error::InvalidFlakePath("Path contains invalid UTF-8".to_string()))?;

    if path_str.is_empty() {
        return Err(Error::InvalidFlakePath(
            "Flake path cannot be empty".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;

    use crate::nix::commands::flake_check;

    #[tokio::test]
    async fn test_run_checks_detailed() {
        let mut flake_path: String = env!("CARGO_MANIFEST_DIR").into();
        flake_path.push('/');
        flake_path.push_str("test-flake");

        let result = flake_check(&flake_path).await.unwrap();

        File::create("test-flake/detailed-log2.json")
            .await
            .unwrap()
            .write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
            .await
            .unwrap();
    }
}
