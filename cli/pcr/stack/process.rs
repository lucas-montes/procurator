use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use chrono::Utc;
use tokio::io::AsyncBufReadExt;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::stack::logging::{LogLine, LogStream, color_for, colored_prefix};
use crate::stack::parser::{Service, ServiceGraph};
use crate::stack::supervisor::{
    RunningService, RunningStack, ServiceHandle, ServiceStatus, ServiceSupervisor, StackState,
    flatten_handles, kill_pid,
};

/// How long to wait after SIGTERM before sending SIGKILL.
pub(crate) const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ProcessSupervisor<S: StackState> {
    pub(crate) repo_root: PathBuf,
    state_repo: S,
    /// Optional sender for file+terminal logging. When `None`, no output routing.
    pub log_sender: Option<mpsc::Sender<LogLine>>,
}

impl<S: StackState> ProcessSupervisor<S> {
    pub fn new(repo_root: PathBuf, state_repo: S) -> Self {
        Self {
            repo_root,
            state_repo,
            log_sender: None,
        }
    }
}

impl<S: StackState> ServiceSupervisor for ProcessSupervisor<S> {
    fn start(&mut self, graph: &ServiceGraph) -> Result<RunningStack, String> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| format!("failed to create tokio runtime: {}", e))?;
        rt.block_on(self.start_impl(graph))
    }

    fn stop(&mut self) -> Result<(), String> {
        let state = self.state_repo.load()?;
        for (name, svc) in &state.services {
            kill_pid(svc.pid, name, GRACEFUL_TIMEOUT);
        }
        self.state_repo.clear()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Spawn helpers
// ---------------------------------------------------------------------------

impl<S: StackState> ProcessSupervisor<S> {
    /// Spawn a single service and return a [`ServiceHandle`].
    ///
    /// For oneShot services this awaits completion: on success the handle
    /// is returned with `pid = 0` / `Status::Stopped`; on failure the error
    /// is propagated (the caller is responsible for cleaning up already-
    /// spawned services).
    async fn spawn_one(&self, name: &str, svc: &Service) -> Result<ServiceHandle, String> {
        let color = color_for(name);
        let prefix = colored_prefix(name, color);
        let is_one_shot = svc.one_shot.unwrap_or(false);

        let (prog, args) = parse_cmd(&svc.cmd)?;

        let mut work_dir = self.repo_root.clone();
        if let Some(src) = &svc.src {
            work_dir.push(src);
        }

        let mut cmd = tokio::process::Command::new(&prog);
        for arg in &args {
            cmd.arg(arg);
        }
        cmd.current_dir(&work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{} failed to spawn: {}", prefix, e))?;

        let pid = child
            .id()
            .ok_or_else(|| format!("{} spawned but PID unavailable", prefix))?;

        println!("{} started (pid {})", prefix, pid);

        let log_tx = self.log_sender.clone();

        // Stream stdout → LogLine channel
        if let Some(stdout) = child.stdout.take() {
            let name = name.to_string();
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            let tx = log_tx.clone();
            tokio::spawn(async move {
                while let Ok(Some(text)) = reader.next_line().await {
                    if let Some(ref tx) = tx {
                        let _ = tx
                            .send(LogLine {
                                service: name.clone(),
                                stream: LogStream::Stdout,
                                text,
                                timestamp: Utc::now(),
                            })
                            .await;
                    }
                }
            });
        }

        // Stream stderr → LogLine channel
        if let Some(stderr) = child.stderr.take() {
            let name = name.to_string();
            let mut reader = tokio::io::BufReader::new(stderr).lines();
            let tx = log_tx.clone();
            tokio::spawn(async move {
                while let Ok(Some(text)) = reader.next_line().await {
                    if let Some(ref tx) = tx {
                        let _ = tx
                            .send(LogLine {
                                service: name.clone(),
                                stream: LogStream::Stderr,
                                text,
                                timestamp: Utc::now(),
                            })
                            .await;
                    }
                }
            });
        }

        let mut running = RunningService {
            cmd: svc.cmd.clone(),
            pid,
            status: ServiceStatus::Running,
        };

        // For oneShot services, wait for completion
        if is_one_shot {
            let exit_status = child
                .wait()
                .await
                .map_err(|e| format!("{} wait error: {}", prefix, e))?;

            if !exit_status.success() {
                return Err(format!(
                    "oneShot service '{}' exited with code {:?}",
                    name,
                    exit_status.code()
                ));
            }

            println!("{} oneShot completed successfully", prefix);
            running.status = ServiceStatus::Stopped;
            running.pid = 0;
        }

        Ok(ServiceHandle {
            name: name.to_string(),
            running,
        })
    }

    /// Spawn services by name, respecting the graph's topological order.
    ///
    /// - `names` is a subset of the service names in `graph`.
    /// - `handles` is mutated in place: new handles are inserted, existing
    ///   handles are left alone (unless `replace` is true, in which case
    ///   the old handle is stopped first).
    /// - State is persisted after each successful spawn.
    pub async fn spawn_many(
        &self,
        names: &[String],
        graph: &ServiceGraph,
        handles: &mut HashMap<String, ServiceHandle>,
        replace: bool,
        stack_pid: u32,
        started_at: &str,
    ) -> Result<(), String> {
        // Preserve topological order: walk graph.order and pick only the
        // requested names.
        let to_spawn: Vec<&str> = graph
            .order
            .iter()
            .filter(|n| names.contains(n))
            .map(|s| s.as_str())
            .collect();

        for svc_name in to_spawn {
            let svc = graph.services.get(svc_name).unwrap();

            // Stop existing handle if replacing.
            if replace {
                if let Some(handle) = handles.get_mut(svc_name) {
                    handle.stop(GRACEFUL_TIMEOUT);
                }
            }

            // Skip already-running services (only relevant when !replace).
            if handles.contains_key(svc_name) {
                continue;
            }

            match self.spawn_one(svc_name, svc).await {
                Ok(handle) => {
                    handles.insert(svc_name.to_string(), handle);
                    // Persist after each successful spawn.
                    let running = flatten_handles(handles, stack_pid, started_at);
                    self.state_repo.save(&running)?;
                }
                Err(e) => {
                    // oneShot or spawn failed — clean up already-spawned
                    // services before propagating.
                    for h in handles.values_mut() {
                        h.stop(GRACEFUL_TIMEOUT);
                    }
                    self.state_repo.clear().ok();
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Spawn **all** services from the graph and return the handle map.
    pub async fn spawn_all(
        &self,
        graph: &ServiceGraph,
    ) -> Result<HashMap<String, ServiceHandle>, String> {
        let stack_pid = std::process::id();
        let started_at = Utc::now().to_rfc3339();
        let mut handles = HashMap::new();
        self.spawn_many(
            &graph.order,
            graph,
            &mut handles,
            false,
            stack_pid,
            &started_at,
        )
        .await?;
        Ok(handles)
    }

    /// Persist the current handle map as running state.
    pub(crate) fn persist_handles(
        &self,
        handles: &HashMap<String, ServiceHandle>,
        stack_pid: u32,
        started_at: &str,
    ) -> Result<(), String> {
        let running = flatten_handles(handles, stack_pid, started_at);
        self.state_repo.save(&running)
    }

    /// Clear persisted state (used during shutdown).
    pub(crate) fn clear_state(&self) -> Result<(), String> {
        self.state_repo.clear()
    }
}

// ---------------------------------------------------------------------------
// start_impl — sync trait wrappers call into this
// ---------------------------------------------------------------------------

impl<S: StackState> ProcessSupervisor<S> {
    pub async fn start_impl(&self, graph: &ServiceGraph) -> Result<RunningStack, String> {
        let stack_pid = std::process::id();
        let started_at = Utc::now().to_rfc3339();

        // ── Install signal handlers (must happen before spawning children) ──
        let mut sigint =
            signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT handler: {}", e))?;
        let mut sigterm =
            signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM handler: {}", e))?;

        // ── Spawn all children ──
        let mut handles = self.spawn_all(graph).await?;

        // If all services are oneShot (already completed), exit immediately.
        let has_running = handles
            .values()
            .any(|h| matches!(h.running.status, ServiceStatus::Running));
        if !has_running {
            println!("All oneShot services completed. Stack exiting.");
            self.state_repo.clear().ok();
            return Ok(flatten_handles(&handles, stack_pid, &started_at));
        }

        println!("All services started. Press Ctrl-C to stop.");

        // ── Wait for shutdown signal ──
        tokio::select! {
            _ = sigint.recv() => {
                println!("\nSIGINT received, shutting down...");
            }
            _ = sigterm.recv() => {
                println!("\nSIGTERM received, shutting down...");
            }
        }

        // ── Graceful shutdown using handles (they always have the latest PIDs) ──
        for h in handles.values_mut() {
            h.stop(GRACEFUL_TIMEOUT);
        }
        self.state_repo.clear().ok();

        println!("Stack stopped.");
        Ok(flatten_handles(&handles, stack_pid, &started_at))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert the JSON cmd value into (program, args).
fn parse_cmd(cmd: &serde_json::Value) -> Result<(String, Vec<String>), String> {
    match cmd {
        serde_json::Value::String(s) => Ok(("sh".to_string(), vec!["-c".to_string(), s.clone()])),
        serde_json::Value::Array(arr) => {
            let mut iter = arr.iter();
            let prog = iter
                .next()
                .and_then(|v| v.as_str())
                .unwrap_or("sh")
                .to_string();
            let args = iter
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            Ok((prog, args))
        }
        _ => Err("invalid cmd: expected string or array".to_string()),
    }
}
