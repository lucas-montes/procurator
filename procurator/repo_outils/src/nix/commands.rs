use serde::{Deserialize, Serialize};
use std::{
    ops::Not,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::{io::BufReader, process::Command};


use super::logs::{self, Error as LogError,Parser, State, Summary};


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

/// Result from `nix build`
#[derive(Debug, Serialize)]
pub struct BuildResult {
    summary: Summary,
    out_paths: Vec<PathBuf>,
    success: bool,
}

/// Result from `nix flake show`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakeShow {
    #[serde(default)]
    description: Option<String>,

    #[serde(default)]
    packages: std::collections::HashMap<String, std::collections::HashMap<String, OutputInfo>>,

    #[serde(default)]
    checks: std::collections::HashMap<String, std::collections::HashMap<String, OutputInfo>>,

    #[serde(default)]
    apps: std::collections::HashMap<String, std::collections::HashMap<String, AppInfo>>,

    #[serde(default, rename = "devShells")]
    dev_shells: std::collections::HashMap<String, std::collections::HashMap<String, OutputInfo>>,

    #[serde(default, rename = "nixosModules")]
    nixos_modules: std::collections::HashMap<String, ModuleInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutputInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    output_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppInfo {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    app_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuleInfo {
    #[serde(default, rename = "type")]
    module_type: Option<String>,
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

/// Run `nix build` - returns summary, out paths, and success status
pub async fn build(
    flake_path: impl AsRef<Path>,
    output_link: Option<impl AsRef<Path>>,
) -> Result<BuildResult> {
    let path = flake_path.as_ref();
    validate_path(path)?;

    let mut command = Command::new("nix");
    command
        .arg("build")
        .arg(path)
        .arg("--print-build-logs")
        .arg("--log-format")
        .arg("internal-json")
        .arg("--json");

    if let Some(link) = output_link {
        command.arg("--out-link").arg(link.as_ref());
    } else {
        command.arg("--no-link");
    }

    command.stderr(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());

    let mut child = command.spawn()?;
    let started_at = std::time::SystemTime::now();

    // Parse logs from stderr
    let stderr = child.stderr.take().ok_or(Error::BuildOutputMissing)?;
    let reader = tokio::io::BufReader::new(stderr);

    let mut state = State::default();
    state.parse_lines(reader).await;

    // Wait for process and get stdout
    let output = child.wait_with_output().await?;
    let completed_at = std::time::SystemTime::now();

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    // Parse output paths from stdout JSON
    #[derive(Deserialize)]
    struct BuildOutput {
        outputs: std::collections::HashMap<String, String>,
    }

    let build_outputs: Vec<BuildOutput> = serde_json::from_slice(&output.stdout)?;
    let out_paths: Vec<PathBuf> = build_outputs
        .into_iter()
        .flat_map(|bo| bo.outputs.into_values())
        .map(PathBuf::from)
        .collect();

    let summary = state.into_output(started_at, completed_at);

    Ok(BuildResult {
        summary,
        out_paths,
        success: true,
    })
}

/// Run `nix flake show` to get flake structure
pub async fn flake_show(flake_path: impl AsRef<Path>) -> Result<FlakeShow> {
    let path = flake_path.as_ref();
    validate_path(path)?;

    let output = Command::new("nix")
        .arg("flake")
        .arg("show")
        .arg(path)
        .arg("--json")
        .output()
        .await?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let show: FlakeShow = serde_json::from_slice(&output.stdout)?;
    Ok(show)
}

/// Run `nix eval` to evaluate a flake attribute and return it as JSON
pub async fn eval_json(flake_ref: impl AsRef<str>) -> Result<serde_json::Value> {
    eval_typed(flake_ref).await
}

/// Run `nix eval` with a specific type (for when you know the structure)
pub async fn eval_typed<T: for<'de> Deserialize<'de>>(flake_ref: impl AsRef<str>) -> Result<T> {
    let flake_ref = flake_ref.as_ref();

    let output = Command::new("nix")
        .arg("eval")
        .arg(flake_ref)
        .arg("--json")
        .output()
        .await?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let value: T = serde_json::from_slice(&output.stdout)?;
    Ok(value)
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
