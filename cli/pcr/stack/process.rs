use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::io::AsyncBufReadExt;
use tokio::signal::unix::{SignalKind, signal};

use crate::stack::parser::ServiceGraph;
use crate::stack::supervisor::{
    RunningService, RunningStack, ServiceStatus, ServiceSupervisor, StackState, is_pid_alive,
};

/// How long to wait after SIGTERM before sending SIGKILL.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Color palette for per-service log prefixes
// ---------------------------------------------------------------------------

/// 30 distinct foreground colors — ANSI 8/16 + 256-color codes.
const COLORS: &[&str] = &[
    "\x1b[31m",       // 0:  red
    "\x1b[32m",       // 1:  green
    "\x1b[33m",       // 2:  yellow
    "\x1b[34m",       // 3:  blue
    "\x1b[35m",       // 4:  magenta
    "\x1b[36m",       // 5:  cyan
    "\x1b[91m",       // 6:  bright red
    "\x1b[92m",       // 7:  bright green
    "\x1b[93m",       // 8:  bright yellow
    "\x1b[94m",       // 9:  bright blue
    "\x1b[95m",       // 10: bright magenta
    "\x1b[96m",       // 11: bright cyan
    "\x1b[38;5;208m", // 12: orange
    "\x1b[38;5;45m",  // 13: sky blue
    "\x1b[38;5;200m", // 14: pink
    "\x1b[38;5;118m", // 15: lime green
    "\x1b[38;5;99m",  // 16: purple
    "\x1b[38;5;37m",  // 17: teal
    "\x1b[38;5;173m", // 18: salmon
    "\x1b[38;5;141m", // 19: lavender
    "\x1b[38;5;48m",  // 20: mint
    "\x1b[38;5;203m", // 21: coral
    "\x1b[38;5;75m",  // 22: cornflower blue
    "\x1b[38;5;220m", // 23: gold
    "\x1b[38;5;205m", // 24: hot pink
    "\x1b[38;5;120m", // 25: pastel green
    "\x1b[38;5;68m",  // 26: steel blue
    "\x1b[38;5;179m", // 27: tan
    "\x1b[38;5;51m",  // 28: bright cyan
    "\x1b[38;5;155m", // 29: chartreuse
];
const RESET: &str = "\x1b[0m";

/// Pick a color from the 30-color palette based on a hash of the service name.
/// This ensures the same service always gets the same color regardless of
/// startup order or how many other services exist.
fn color_for(name: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = hasher.finish() as usize % COLORS.len();
    COLORS[idx]
}

/// Build a service-name → ANSI color-code lookup from the boot order.
fn build_color_map(order: &[String]) -> HashMap<String, String> {
    order
        .iter()
        .map(|name| (name.clone(), color_for(name).to_string()))
        .collect()
}

/// Format a log prefix like `[service_name]` with the service's color.
fn colored_prefix(name: &str, color: &str) -> String {
    format!("{}[{}]{}", color, name, RESET)
}

// ---------------------------------------------------------------------------
// Adapter: process-based ServiceSupervisor
// ---------------------------------------------------------------------------

pub struct ProcessSupervisor<S: StackState> {
    repo_root: PathBuf,
    state_repo: S,
}

impl<S: StackState> ProcessSupervisor<S> {
    pub fn new(repo_root: PathBuf, state_repo: S) -> Self {
        Self {
            repo_root,
            state_repo,
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
        let colors: HashMap<String, String> = state
            .services
            .keys()
            .map(|name| (name.clone(), color_for(name).to_string()))
            .collect();
        kill_service_pids(&state.services, &colors, GRACEFUL_TIMEOUT);
        self.state_repo.clear()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl<S: StackState> ProcessSupervisor<S> {
    pub async fn start_impl(&self, graph: &ServiceGraph) -> Result<RunningStack, String> {
        // ── Install signal handlers (must happen before spawning children) ──
        let mut sigint =
            signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT handler: {}", e))?;
        let mut sigterm =
            signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM handler: {}", e))?;

        // ── Assign a color to each service ──
        let colors = build_color_map(&graph.order);

        // ── Spawn children in topological order ──
        let mut running = RunningStack {
            version: 1,
            stack_pid: std::process::id(),
            started_at: Utc::now().to_rfc3339(),
            services: HashMap::new(),
        };

        for svc_name in &graph.order {
            let svc = graph.services.get(svc_name).unwrap();
            let color = colors.get(svc_name).map(|s| s.as_str()).unwrap_or("");
            let prefix = colored_prefix(svc_name, color);

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

            // Stream stdout
            if let Some(stdout) = child.stdout.take() {
                let name = svc_name.clone();
                let color_code = color.to_string();
                let mut reader = tokio::io::BufReader::new(stdout).lines();
                tokio::spawn(async move {
                    let p = colored_prefix(&name, &color_code);
                    while let Ok(Some(line)) = reader.next_line().await {
                        println!("{} {}", p, line);
                    }
                });
            }

            // Stream stderr
            if let Some(stderr) = child.stderr.take() {
                let name = svc_name.clone();
                let color_code = color.to_string();
                let mut reader = tokio::io::BufReader::new(stderr).lines();
                tokio::spawn(async move {
                    let p = colored_prefix(&name, &color_code);
                    while let Ok(Some(line)) = reader.next_line().await {
                        eprintln!("{} ERR {}", p, line);
                    }
                });
            }

            running.services.insert(
                svc_name.clone(),
                RunningService {
                    cmd: svc.cmd.clone(),
                    pid,
                    status: ServiceStatus::Running,
                },
            );

            // For oneShot services, wait for completion and check exit code.
            if is_one_shot {
                let exit_status = child
                    .wait()
                    .await
                    .map_err(|e| format!("{} wait error: {}", prefix, e))?;

                if !exit_status.success() {
                    // oneShot failed — abort the whole stack.
                    eprintln!(
                        "{} oneShot failed with exit code {:?}; aborting stack",
                        prefix,
                        exit_status.code()
                    );
                    // Kill any services that were already started.
                    kill_service_pids(&running.services, &colors, GRACEFUL_TIMEOUT);
                    self.state_repo.clear().ok();
                    return Err(format!(
                        "oneShot service '{}' exited with code {:?}",
                        svc_name,
                        exit_status.code()
                    ));
                }

                println!("{} oneShot completed successfully", prefix);
                // Mark as stopped (it already exited).
                if let Some(entry) = running.services.get_mut(svc_name) {
                    entry.status = ServiceStatus::Stopped;
                    entry.pid = 0;
                }
            } else {
                // Regular service — persist state with the new PID.
                self.state_repo.save(&running)?;
            }
        }

        // Persist final state before entering the wait loop.
        self.state_repo.save(&running)?;

        // If all services are oneShot (already completed), exit immediately.
        let has_running = running
            .services
            .values()
            .any(|s| matches!(s.status, ServiceStatus::Running));
        if !has_running {
            println!("All oneShot services completed. Stack exiting.");
            self.state_repo.clear().ok();
            return Ok(running);
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

        // ── Graceful shutdown ──
        // Reload state to get the latest PIDs.
        if let Ok(latest) = self.state_repo.load() {
            kill_service_pids(&latest.services, &colors, GRACEFUL_TIMEOUT);
        } else {
            // If state is already gone, still try children from our local map.
            kill_service_pids(&running.services, &colors, GRACEFUL_TIMEOUT);
        }
        self.state_repo.clear().ok();

        println!("Stack stopped.");
        Ok(running)
    }
}

// ---------------------------------------------------------------------------
// Utility: signal + kill with graceful escalation
// ---------------------------------------------------------------------------

/// Send SIGTERM to all services, wait up to `timeout`, then SIGKILL survivors.
fn kill_service_pids(
    services: &HashMap<String, RunningService>,
    colors: &HashMap<String, String>,
    timeout: Duration,
) {
    // Phase 1: SIGTERM
    for (name, svc) in services.iter() {
        if svc.pid != 0 && is_pid_alive(svc.pid) {
            let ok = std::process::Command::new("kill")
                .arg(svc.pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                let color = colors.get(name).map(|s| s.as_str()).unwrap_or("");
                println!("{} SIGTERM sent", colored_prefix(name, color));
            }
        }
    }

    // Phase 2: wait for processes to die (polling)
    let deadline = Instant::now() + timeout;
    loop {
        let all_dead = services
            .iter()
            .all(|(_, svc)| svc.pid == 0 || !is_pid_alive(svc.pid));
        if all_dead || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // Phase 3: SIGKILL survivors
    for (name, svc) in services.iter() {
        if svc.pid != 0 && is_pid_alive(svc.pid) {
            let _ = std::process::Command::new("kill")
                .arg("-9")
                .arg(svc.pid.to_string())
                .status();
            let color = colors.get(name).map(|s| s.as_str()).unwrap_or("");
            eprintln!("{} force killed", colored_prefix(name, color));
        }
    }
}

// ---------------------------------------------------------------------------
// Utility: command parsing
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
