use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::stack::parser::ServiceGraph;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Serializable running state for a single service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningService {
    /// The command that was launched (for reference / re-start).
    pub cmd: serde_json::Value,
    /// OS PID of the child process (0 if not running).
    pub pid: u32,
    /// Current status.
    pub status: ServiceStatus,
}

/// Current lifecycle status of a service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Failed,
}

/// The complete persisted state of a running stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningStack {
    pub version: u32,
    /// PID of the `pcr stack start` process that owns the children.
    pub stack_pid: u32,
    /// ISO-8601 timestamp of when the stack was started.
    pub started_at: String,
    /// Per-service running state, keyed by service name.
    pub services: HashMap<String, RunningService>,
}

// ---------------------------------------------------------------------------
// Port: persistence of running stack state
// ---------------------------------------------------------------------------

pub trait StackState: Send + Sync {
    /// Persist the current running state.
    fn save(&self, state: &RunningStack) -> Result<(), String>;

    /// Load a previously-persisted running state.
    fn load(&self) -> Result<RunningStack, String>;

    /// Remove the persisted state entirely (clean shutdown).
    fn clear(&self) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// Port: process supervision
// ---------------------------------------------------------------------------

pub trait ServiceSupervisor: Send + Sync {
    /// Start all services declared in the graph and return the running state.
    fn start(&mut self, graph: &ServiceGraph) -> Result<RunningStack, String>;

    /// Stop all running services and clear state.
    fn stop(&mut self) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// Adapter: file-based StackState
// ---------------------------------------------------------------------------

/// File-based implementation of [`StackState`].
///
/// Persists the running stack as JSON at `<root>/.pcr-stack/state.json`.
/// Uses `fs2` advisory file locking to prevent concurrent writes.
pub struct FileStackState {
    root: PathBuf,
}

impl FileStackState {
    /// Create a new file-backed state store rooted at `repo_root`.
    pub fn new(repo_root: PathBuf) -> Self {
        Self { root: repo_root }
    }

    /// Full path to the state directory.
    fn state_dir(&self) -> PathBuf {
        self.root.join(".pcr-stack")
    }

    /// Full path to the state JSON file.
    fn state_path(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }
}

impl StackState for FileStackState {
    fn save(&self, state: &RunningStack) -> Result<(), String> {
        let dir = self.state_dir();
        fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create state dir {:?}: {}", dir, e))?;

        let path = self.state_path();

        // Acquire exclusive lock on the state file.
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&path)
            .map_err(|e| format!("failed to open state file {:?}: {}", path, e))?;
        file.try_lock_exclusive().map_err(|_| {
            "another process holds the stack lock (is `pcr stack start` already running?)"
                .to_string()
        })?;

        // Write to a temp file first for crash safety, then rename.
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| format!("serialization error: {}", e))?;
        let tmp_path = dir.join("state.json.tmp");
        {
            let mut tmp = fs::File::create(&tmp_path)
                .map_err(|e| format!("failed to create temp file {:?}: {}", tmp_path, e))?;
            tmp.write_all(json.as_bytes())
                .map_err(|e| format!("failed to write temp file: {}", e))?;
            tmp.sync_all()
                .map_err(|e| format!("failed to sync temp file: {}", e))?;
        }
        fs::rename(&tmp_path, &path).map_err(|e| format!("failed to rename state file: {}", e))?;

        // Release the lock by letting the file handle drop.
        drop(file);
        Ok(())
    }

    fn load(&self) -> Result<RunningStack, String> {
        let path = self.state_path();

        if !path.exists() {
            return Err("no stack state file found (stack is not running)".to_string());
        }

        let file = fs::File::open(&path)
            .map_err(|e| format!("failed to open state file {:?}: {}", path, e))?;
        file.try_lock_exclusive().map_err(|_| {
            "another process holds the stack lock (is `pcr stack start` already running?)"
                .to_string()
        })?;

        let state: RunningStack = serde_json::from_reader(&file)
            .map_err(|e| format!("failed to parse state file: {}", e))?;

        // Release the lock — we only needed it for the read.
        drop(file);

        // Check for stale state: verify the stack PID is alive.
        let pid_alive = is_pid_alive(state.stack_pid);
        if !pid_alive {
            // Stale state — warn and clean up.
            eprintln!(
                "warning: stack PID {} is not running; cleaning up stale state",
                state.stack_pid
            );
            let _ = self.clear();
            return Err("stale stack state cleaned up (stack PID no longer alive)".to_string());
        }

        Ok(state)
    }

    fn clear(&self) -> Result<(), String> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(());
        }

        // Try to lock before removing to avoid races.
        if let Ok(file) = fs::File::open(&path) {
            let _ = file.try_lock_exclusive();
            // Ignore lock errors on clear — we want to clean up aggressively.
        }

        fs::remove_file(&path).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(()) // already gone
            } else {
                Err(format!("failed to remove state file: {}", e))
            }
        })?;

        // Also try to clean up the (now empty) directory.
        let _ = fs::remove_dir(self.state_dir());

        Ok(())
    }
}

/// Check whether `pid` is currently alive by running `kill -0 <pid>`.
///
/// Signal 0 tests whether the process exists without actually sending a signal.
pub(crate) fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
