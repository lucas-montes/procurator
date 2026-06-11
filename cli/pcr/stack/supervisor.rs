use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::stack::logging::ColoredPrefix;
use crate::stack::parser::ServiceGraph;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningService {
    cmd: serde_json::Value,
    pid: u32,
    status: ServiceStatus,
}

impl RunningService {
    pub fn new(cmd: serde_json::Value, pid: u32, status: ServiceStatus) -> Self {
        Self { cmd, pid, status }
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn status(&self) -> &ServiceStatus {
        &self.status
    }
    pub fn cmd(&self) -> &serde_json::Value {
        &self.cmd
    }
    pub fn set_pid(&mut self, pid: u32) {
        self.pid = pid;
    }
    pub fn set_status(&mut self, status: ServiceStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningStack {
    version: u32,
    stack_pid: u32,
    started_at: String,
    services: HashMap<String, RunningService>,
}

impl RunningStack {
    pub fn services(&self) -> &HashMap<String, RunningService> {
        &self.services
    }
}

pub struct ServiceHandle {
    name: String,
    running: RunningService,
}

impl ServiceHandle {
    pub fn new(name: String, running: RunningService) -> Self {
        Self { name, running }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn running(&self) -> &RunningService {
        &self.running
    }

    pub fn stop(&mut self, timeout: Duration) {
        if self.running.pid == 0 {
            return;
        }
        kill_pid(self.running.pid, &self.name, timeout);
        self.running.pid = 0;
        self.running.status = ServiceStatus::Stopped;
    }
}

// ---------------------------------------------------------------------------
// SupervisorError
// ---------------------------------------------------------------------------

/// Errors from stack state persistence and supervision.
#[derive(Debug)]
pub enum SupervisorError {
    Io(std::io::Error),
    Json(serde_json::Error),
    LockTaken,
    StaleState {
        stack_pid: u32,
    },
    /// Generic process-level error (spawn, signal, etc.).
    Process(String),
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SupervisorError::Io(e) => write!(f, "state I/O error: {}", e),
            SupervisorError::Json(e) => write!(f, "state JSON error: {}", e),
            SupervisorError::LockTaken => {
                write!(f, "another process holds the stack lock")
            }
            SupervisorError::StaleState { stack_pid } => {
                write!(f, "stale state: stack PID {} is no longer alive", stack_pid)
            }
            SupervisorError::Process(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for SupervisorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SupervisorError::Io(e) => Some(e),
            SupervisorError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SupervisorError {
    fn from(e: std::io::Error) -> Self {
        SupervisorError::Io(e)
    }
}

impl From<serde_json::Error> for SupervisorError {
    fn from(e: serde_json::Error) -> Self {
        SupervisorError::Json(e)
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

pub trait StackState: Send + Sync {
    fn save(&self, state: &RunningStack) -> Result<(), SupervisorError>;
    fn load(&self) -> Result<RunningStack, SupervisorError>;
    fn clear(&self) -> Result<(), SupervisorError>;
}

pub trait ServiceSupervisor: Send + Sync {
    fn start(&mut self, graph: &ServiceGraph) -> Result<RunningStack, SupervisorError>;
    fn stop(&mut self) -> Result<(), SupervisorError>;
}

// ---------------------------------------------------------------------------
// FileStackState
// ---------------------------------------------------------------------------

pub struct FileStackState {
    root: PathBuf,
}

impl FileStackState {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { root: repo_root }
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join(".pcr-stack")
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }
}

impl StackState for FileStackState {
    fn save(&self, state: &RunningStack) -> Result<(), SupervisorError> {
        let dir = self.state_dir();
        fs::create_dir_all(&dir)?;

        let path = self.state_path();

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(&path)?;
        file.try_lock_exclusive()
            .map_err(|_| SupervisorError::LockTaken)?;

        let json = serde_json::to_string_pretty(state)?;
        let tmp_path = dir.join("state.json.tmp");
        {
            let mut tmp = fs::File::create(&tmp_path)?;
            tmp.write_all(json.as_bytes())?;
            tmp.sync_all()?;
        }
        fs::rename(&tmp_path, &path)?;

        drop(file);
        Ok(())
    }

    fn load(&self) -> Result<RunningStack, SupervisorError> {
        let path = self.state_path();

        if !path.exists() {
            return Err(SupervisorError::Process(
                "no stack state file found (stack is not running)".to_string(),
            ));
        }

        let file = fs::File::open(&path)?;
        file.try_lock_exclusive()
            .map_err(|_| SupervisorError::LockTaken)?;

        let state: RunningStack = serde_json::from_reader(&file)?;
        drop(file);

        if !is_pid_alive(state.stack_pid) {
            eprintln!(
                "warning: stack PID {} is not running; cleaning up stale state",
                state.stack_pid
            );
            let _ = self.clear();
            return Err(SupervisorError::StaleState {
                stack_pid: state.stack_pid,
            });
        }

        Ok(state)
    }

    fn clear(&self) -> Result<(), SupervisorError> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(());
        }

        if let Ok(file) = fs::File::open(&path) {
            let _ = file.try_lock_exclusive();
        }

        fs::remove_file(&path).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(SupervisorError::Io(e))
            }
        })?;

        let _ = fs::remove_dir(self.state_dir()).map_err(SupervisorError::Io)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn is_pid_alive(pid: u32) -> bool {
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

pub fn kill_pid(pid: u32, name: &str, timeout: Duration) {
    if pid == 0 || !is_pid_alive(pid) {
        return;
    }

    let ok = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("{} SIGTERM sent", ColoredPrefix::new(name));
    }

    let deadline = Instant::now() + timeout;
    loop {
        if !is_pid_alive(pid) || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    if is_pid_alive(pid) {
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status();
        eprintln!("{} force killed", ColoredPrefix::new(name));
    }
}

pub fn flatten_handles(
    handles: &HashMap<String, ServiceHandle>,
    stack_pid: u32,
    started_at: &str,
) -> RunningStack {
    let services = handles
        .iter()
        .map(|(name, h)| (name.clone(), h.running.clone()))
        .collect();
    RunningStack {
        version: 1,
        stack_pid,
        started_at: started_at.to_string(),
        services,
    }
}
