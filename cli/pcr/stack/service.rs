use core::fmt;
use std::{
    collections::HashMap,
    path::Path,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead},
    process::Child,
    sync::mpsc::Sender,
};

use crate::stack::config;
use crate::stack::health::{HealthCheckError, run_healthcheck};
use crate::stack::logging::{LogLine, LogStream};
use crate::stack::watch::WatchEvent;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Spawn(String),
    Wait(String),
    Signal(String),
    ParseCmd,
    Config(config::ParserError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::Spawn(msg) => write!(f, "{}", msg),
            Error::Wait(msg) => write!(f, "{}", msg),
            Error::Signal(msg) => write!(f, "{}", msg),
            Error::ParseCmd => write!(f, "invalid cmd: expected string or array"),
            Error::Config(e) => write!(f, "config error: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Config(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<config::ParserError> for Error {
    fn from(e: config::ParserError) -> Self {
        Error::Config(e)
    }
}

// ---------------------------------------------------------------------------
// Service type states
// ---------------------------------------------------------------------------

pub struct Parsed {
    name: String,
    config: config::Service,
    working_dir: PathBuf,
}

impl Clone for Parsed {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            config: self.config.clone(),
            working_dir: self.working_dir.clone(),
        }
    }
}

pub struct Running {
    handle: Child,
}

/// A service is a cmd to run, it can be either a one-shot or a long running one
pub struct Service<State = Parsed>(State);

impl<State: Clone> Clone for Service<State> {
    fn clone(&self) -> Self {
        Service(self.0.clone())
    }
}

impl Service<Running> {
    pub async fn kill(mut self) -> Result<(), Error> {
        self.0
            .handle
            .kill()
            .await
            .map_err(|e| Error::Signal(e.to_string()))?;
        let output = self
            .0
            .handle
            .wait_with_output()
            .await
            .map_err(|e| Error::Wait(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(stderr = %stderr, "service exited with non-zero status");
        }
        Ok(())
    }
}

impl Service {
    pub fn new(metadata: Parsed) -> Self {
        Self(metadata)
    }

    /// Access the parsed config (restart policy, cmd, etc.).
    pub fn config(&self) -> &config::Service {
        &self.0.config
    }

    /// Working directory for this service (resolved from `src`).
    #[must_use]
    pub fn working_dir(&self) -> &Path {
        &self.0.working_dir
    }

    pub fn start(self, logger: Sender<LogLine>) -> Result<Service<Running>, Error> {
        let (prog, args) = parse_cmd(self.0.config.cmd())?;

        let mut cmd = tokio::process::Command::new(&prog);
        cmd.kill_on_drop(true);

        // On Linux, ask the kernel to SIGTERM the child if this parent dies
        // (handles SIGKILL where drop doesn't run).
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: pre_exec runs in the child after fork, before exec.
            // The closure is async-signal-safe (single prctl syscall, no heap).
            unsafe {
                cmd.as_std_mut().pre_exec(|| linux_pdeathsig::apply());
            }
        }

        let mut child = cmd
            .args(&args)
            .current_dir(&self.0.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Spawn(format!("failed to spawn: {e}")))?;

        let stdout = child.stdout.take().expect("stdout should be piped");
        spawn_logger(
            self.0.name.clone(),
            stdout,
            logger.clone(),
            LogStream::Stdout,
        );

        let stderr = child.stderr.take().expect("stderr should be piped");
        spawn_logger(self.0.name, stderr, logger, LogStream::Stderr);

        Ok(Service::<Running>(Running { handle: child }))
    }
}

// ---------------------------------------------------------------------------
// ServiceManifest — resolved config, ready to run (the currency type)
// ---------------------------------------------------------------------------

/// A fully resolved set of services ready to be spawned.
/// This is the intermediate type between config and Supervisor:
/// config → ServiceManifest → Supervisor → running services.
#[derive(Clone)]
pub struct ServiceManifest {
    services: HashMap<String, Service<Parsed>>,
    order: Vec<String>,
}

impl ServiceManifest {
    /// Build a manifest from a parsed config graph and repo path.
    /// Resolves `working_dir` from each service's `src` field.
    pub fn from_graph(graph: &config::ServiceGraph, repo_path: &Path) -> Self {
        let services = graph
            .services()
            .iter()
            .map(|(name, svc)| {
                let working_dir = svc
                    .src()
                    .map(|s| repo_path.join(s))
                    .unwrap_or_else(|| repo_path.to_path_buf());
                let parsed = Parsed {
                    name: name.clone(),
                    config: svc.clone(),
                    working_dir,
                };
                (name.clone(), Service::new(parsed))
            })
            .collect();

        Self {
            services,
            order: graph.order().to_vec(),
        }
    }

    /// Diff this manifest against a newer one. Returns a map of service names
    /// to their change classification (Added / Removed / Changed / Unchanged).
    pub fn diff(&self, other: &Self) -> HashMap<String, config::ServiceChange> {
        let mut result: HashMap<String, config::ServiceChange> = HashMap::new();

        for name in self.order.iter() {
            let change = match other.services.get(name) {
                None => config::ServiceChange::Removed,
                Some(other_svc) => {
                    // Compare by config.cmd (same logic as config::diff_graphs)
                    if self.services[name].0.config.cmd() == other_svc.0.config.cmd() {
                        config::ServiceChange::Unchanged
                    } else {
                        config::ServiceChange::Changed
                    }
                }
            };
            result.insert(name.clone(), change);
        }

        for name in other.order.iter() {
            if !self.services.contains_key(name) {
                result.insert(name.clone(), config::ServiceChange::Added);
            }
        }

        result
    }

    /// Start every service in dependency order. Returns a RunningManifest
    /// that owns all child process handles.
    ///
    /// NOTE: Dead code — replaced by `Supervisor::start_services_with_healthchecks()`
    /// (line ~501). This method does not support healthcheck blocking or the
    /// `is_alive()` false-positive guard. Keep for reference until the migration
    /// is confirmed stable, then remove.
    pub fn start_all(mut self, logs_tx: Sender<LogLine>) -> Result<RunningManifest, Error> {
        let mut services: HashMap<String, Service<Running>> = HashMap::new();
        for name in &self.order {
            let svc = self.services.remove(name).expect("service in manifest");
            let running = svc.start(logs_tx.clone())?;
            services.insert(name.clone(), running);
        }
        Ok(RunningManifest { services })
    }

    pub fn order(&self) -> &[String] {
        &self.order
    }

    /// Look up a parsed service by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Service<Parsed>> {
        self.services.get(name)
    }
}

// ---------------------------------------------------------------------------
// RunningManifest — set of running child processes
// ---------------------------------------------------------------------------

pub struct RunningManifest {
    services: HashMap<String, Service<Running>>,
}

impl RunningManifest {
    /// Create an empty manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Kill all services in reverse dependency order.
    pub async fn kill_all(&mut self, order: &[String]) {
        for name in order.iter().rev() {
            if let Some(svc) = self.services.remove(name) {
                tracing::info!(name = %name, "shutting down service");
                if let Err(e) = svc.kill().await {
                    tracing::error!(name = %name, error = %e, "failed to kill service during shutdown");
                }
            }
        }
    }

    /// Remove and kill a single service by name.
    pub async fn remove(&mut self, name: &str) {
        if let Some(svc) = self.services.remove(name) {
            tracing::info!(name = %name, "stopping service");
            if let Err(e) = svc.kill().await {
                tracing::error!(name = %name, error = %e, "failed to kill service");
            }
        }
    }

    /// Insert a running service.
    pub fn insert(&mut self, name: String, svc: Service<Running>) {
        self.services.insert(name, svc);
    }

    /// Check if a service is still running (process alive).
    /// Returns `false` if the service is not in the manifest or has exited.
    pub fn is_alive(&mut self, name: &str) -> bool {
        self.services
            .get_mut(name)
            .map_or(false, |svc| matches!(svc.0.handle.try_wait(), Ok(None)))
    }

    /// Poll every running service. Returns the names and exit statuses of
    /// services that have exited. Removes them from the set.
    pub fn check_health(&mut self) -> Vec<(String, ExitStatus)> {
        let mut dead = Vec::new();
        self.services.retain(|name, svc| {
            match svc.0.handle.try_wait() {
                Ok(Some(status)) => {
                    dead.push((name.clone(), status));
                    false // exited — remove
                }
                Ok(None) => true, // still alive
                Err(e) => {
                    tracing::warn!(name = %name, error = %e, "health check error, will retry");
                    true // transient error — keep for now
                }
            }
        });
        dead
    }
}

// ---------------------------------------------------------------------------
// Supervisor — event loop
// ---------------------------------------------------------------------------

pub struct Supervisor {
    manifest: ServiceManifest,
    logs_tx: Sender<LogLine>,
    watcher_rx: tokio::sync::mpsc::Receiver<WatchEvent>,
    /// Per-service stop signals for background healthcheck loops.
    healthcheck_stop: HashMap<String, Arc<tokio::sync::Notify>>,
}

impl Supervisor {
    pub fn new(
        manifest: ServiceManifest,
        logs_tx: Sender<LogLine>,
        watcher_rx: tokio::sync::mpsc::Receiver<WatchEvent>,
    ) -> Self {
        Self {
            manifest,
            logs_tx,
            watcher_rx,
            healthcheck_stop: HashMap::new(),
        }
    }

    /// Start all services from the initial manifest, then enter the event loop.
    /// On Ctrl-C, clean shutdown. On watcher signal, diff and restart.
    pub async fn run(mut self) -> Result<(), Error> {
        // Phase 1: Start services with dependency healthcheck blocking.
        // Wrap in a select! so Ctrl-C works even during blocking healthcheck waits.
        let mut running = {
            let startup = self.start_services_with_healthchecks();
            tokio::select! {
                result = startup => {
                    result?
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received SIGINT during startup, shutting down");
                    return Ok(());
                }
            }
        };

        // Phase 2: Spawn periodic healthcheck loops for configured services.
        self.spawn_periodic_healthchecks(&mut running);

        // Phase 3: Event loop.
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received SIGINT, shutting down");
                    break;
                }
                Some(event) = self.watcher_rx.recv() => {
                    match event {
                        WatchEvent::ConfigChanged(new_manifest) => {
                            // Stop healthcheck loops for services that are removed or changed.
                            for name in self.manifest.order() {
                                let change = match new_manifest.services.get(name) {
                                    None => true,  // removed
                                    Some(new_svc) => {
                                        new_svc.config().cmd()
                                            != self.manifest.get(name).unwrap().config().cmd()
                                    }  // changed
                                };
                                if change {
                                    if let Some(stop) = self.healthcheck_stop.remove(name) {
                                        stop.notify_one();
                                    }
                                }
                            }
                            self.apply_manifest(&new_manifest, &mut running).await;
                            self.manifest = new_manifest;
                            // Start healthcheck loops for added/changed services.
                            for name in self.manifest.order() {
                                if !self.healthcheck_stop.contains_key(name) {
                                    if let Some(svc) = self.manifest.get(name) {
                                        if let Some(hc) = svc.config().healthcheck() {
                                            let stop = Arc::new(tokio::sync::Notify::new());
                                            spawn_healthcheck_loop(
                                                name.clone(),
                                                hc.clone(),
                                                svc.working_dir().to_path_buf(),
                                                self.logs_tx.clone(),
                                                stop.clone(),
                                            );
                                            self.healthcheck_stop.insert(name.clone(), stop);
                                        }
                                    }
                                }
                            }
                        }
                        WatchEvent::SourceChanged(names) => {
                            for name in &names {
                                tracing::info!(name = %name, "restarted by source change");
                                running.remove(name).await;
                                if let Some(svc) = self.manifest.services.get(name) {
                                    let name = name.clone();
                                    match svc.clone().start(self.logs_tx.clone()) {
                                        Ok(running_svc) => {
                                            running.insert(name, running_svc);
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                name = %name,
                                                error = %e,
                                                "failed to restart service after source change"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    let dead = running.check_health();
                    for (name, status) in dead {
                        tracing::warn!(name = %name, exit = %status, "service exited unexpectedly");
                        // Stop healthcheck loop for this service.
                        if let Some(stop) = self.healthcheck_stop.remove(&name) {
                            stop.notify_one();
                        }
                        // Check restart policy — only restart if explicitly configured.
                        if let Some(svc) = self.manifest.services.get(&name) {
                            if svc.config().restart() == Some("always") {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                match svc.clone().start(self.logs_tx.clone()) {
                                    Ok(running_svc) => {
                                        tracing::info!(name = %name, "service restarted");
                                        running.insert(name, running_svc);
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            name = %name,
                                            error = %e,
                                            "failed to restart service"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Stop all remaining healthcheck loops.
        for (_, stop) in self.healthcheck_stop.drain() {
            stop.notify_one();
        }
        running.kill_all(self.manifest.order()).await;
        Ok(())
    }

    /// Start services in dependency order, blocking on dependency healthchecks.
    async fn start_services_with_healthchecks(&self) -> Result<RunningManifest, Error> {
        let mut running = RunningManifest::new();
        for name in self.manifest.order() {
            let svc = self.manifest.get(name).expect("service in manifest order");

            // Wait for each dependency's healthcheck before starting.
            if let Some(deps) = svc.config().depends_on() {
                for dep_name in deps {
                    if let Some(dep_svc) = self.manifest.get(dep_name) {
                        if let Some(hc) = dep_svc.config().healthcheck() {
                            tracing::info!(
                                service = %name,
                                dependency = %dep_name,
                                "waiting for dependency healthcheck"
                            );
                            let dep_dir = dep_svc.working_dir().to_path_buf();
                            let hc_passed = wait_for_dependency_health(dep_name, hc, &dep_dir)
                                .await
                                .is_ok();
                            if hc_passed {
                                // Healthcheck passed, but verify the service process is
                                // still alive. A false positive occurs when a stale
                                // process holds the port while the real service is dead.
                                if !running.is_alive(dep_name) {
                                    tracing::error!(
                                        service = %name,
                                        dependency = %dep_name,
                                        "dependency healthcheck passed but process exited \
                                         (possible false positive — stale port/handle \
                                         from another process)"
                                    );
                                }
                            } else {
                                tracing::error!(
                                    service = %name,
                                    dependency = %dep_name,
                                    "dependency healthcheck never passed, starting anyway"
                                );
                            }
                        }
                    }
                }
            }

            let running_svc = svc.clone().start(self.logs_tx.clone())?;
            running.insert(name.clone(), running_svc);
        }
        Ok(running)
    }

    /// Spawn a background healthcheck loop for every service that has a
    /// healthcheck configured. Each loop gets a `Notify` that can be used
    /// to cancel it when the service exits or is removed.
    fn spawn_periodic_healthchecks(&mut self, _running: &mut RunningManifest) {
        for name in self.manifest.order() {
            let svc = self.manifest.get(name).expect("service in manifest order");
            if let Some(hc) = svc.config().healthcheck() {
                let stop = Arc::new(tokio::sync::Notify::new());
                spawn_healthcheck_loop(
                    name.clone(),
                    hc.clone(),
                    svc.working_dir().to_path_buf(),
                    self.logs_tx.clone(),
                    stop.clone(),
                );
                self.healthcheck_stop.insert(name.clone(), stop);
            }
        }
    }

    async fn apply_manifest(&self, new_manifest: &ServiceManifest, running: &mut RunningManifest) {
        let diff = self.manifest.diff(new_manifest);

        // Kill removed/changed in reverse order so dependents die first
        for name in self.manifest.order().iter().rev() {
            match diff.get(name) {
                Some(config::ServiceChange::Removed) | Some(config::ServiceChange::Changed) => {
                    running.remove(name).await;
                }
                _ => {}
            }
        }

        // Start new/changed in dependency order
        for name in new_manifest.order() {
            match diff.get(name) {
                Some(config::ServiceChange::Added) | Some(config::ServiceChange::Changed) => {
                    if let Some(svc) = new_manifest.services.get(name) {
                        let name = name.clone();
                        tracing::info!(name = %name, "starting service");
                        match svc.clone().start(self.logs_tx.clone()) {
                            Ok(running_svc) => {
                                running.insert(name, running_svc);
                            }
                            Err(e) => {
                                tracing::error!(name = %name, error = %e, "failed to start service");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn spawn_logger(
    name: String,
    std_buffer: impl AsyncRead + Unpin + 'static + Send,
    logger: Sender<LogLine>,
    stream_side: LogStream,
) {
    tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(std_buffer).lines();
        while let Ok(Some(text)) = reader.next_line().await {
            let _ = logger
                .send(LogLine::new(
                    name.clone(),
                    stream_side.clone(),
                    text,
                    Utc::now(),
                ))
                .await;
        }
    });
}

/// NOTE: Related to `parse_test()` in health.rs (line 50). Both extract
/// `(prog, args)` from `&serde_json::Value` with identical string→`sh -c`
/// and array→direct-exec logic. `parse_test` is a superset that adds
/// `CMD-SHELL` and `CMD` prefix conventions for Docker Compose compatibility.
/// Consider extracting a shared base parser if both continue to evolve.
fn parse_cmd(cmd: &serde_json::Value) -> Result<(String, Vec<String>), Error> {
    match cmd {
        serde_json::Value::String(s) => Ok(("sh".to_string(), vec!["-c".to_string(), s.clone()])),
        serde_json::Value::Array(arr) => {
            let mut iter = arr.into_iter();
            let prog = iter
                .next()
                .and_then(|v| v.as_str())
                .unwrap_or("sh")
                .to_string();
            let args: Vec<String> = iter
                .map(|v| v.as_str().ok_or(Error::ParseCmd).map(|s| s.to_string()))
                .collect::<Result<_, _>>()?;
            Ok((prog, args))
        }
        _ => Err(Error::ParseCmd),
    }
}

/// Poll a dependency's healthcheck up to `retries` times.
///
/// Each attempt is subject to `timeout_secs`. Sleeps `interval_secs` between
/// attempts. Logs each attempt and returns `Ok(())` on the first passing check.
async fn wait_for_dependency_health(
    dep_name: &str,
    config: &config::HealthCheckConfig,
    working_dir: &Path,
) -> Result<(), ()> {
    let retries = config.retries();
    for attempt in 1..=retries {
        match run_healthcheck(config, working_dir).await {
            Ok(()) => {
                tracing::info!(name = %dep_name, "healthcheck: passed");
                return Ok(());
            }
            Err(HealthCheckError::Timeout) => {
                tracing::warn!(
                    name = %dep_name,
                    attempt,
                    max_retries = retries,
                    "healthcheck: timed out"
                );
            }
            Err(e) => {
                tracing::warn!(
                    name = %dep_name,
                    attempt,
                    max_retries = retries,
                    error = %e,
                    "healthcheck: failed"
                );
            }
        }
        if attempt < retries {
            tokio::time::sleep(Duration::from_secs(config.interval_secs())).await;
        }
    }
    tracing::error!(name = %dep_name, retries, "healthcheck: all retries exhausted");
    Err(())
}

/// Spawn a background task that runs a periodic healthcheck for a service.
///
/// Logs state transitions (healthy→unhealthy, unhealthy→healthy) via the
/// `LogLine` channel so they appear with the `[name]` prefix in output.
/// The loop exits when `stop` is notified (service exited or removed).
fn spawn_healthcheck_loop(
    name: String,
    config: config::HealthCheckConfig,
    working_dir: PathBuf,
    logs_tx: Sender<LogLine>,
    stop: Arc<tokio::sync::Notify>,
) {
    tokio::spawn(async move {
        let mut was_healthy = true; // assume healthy before first check
        loop {
            tokio::select! {
                _ = stop.notified() => {
                    return;
                }
                _ = tokio::time::sleep(Duration::from_secs(config.interval_secs())) => {}
            }
            let is_healthy = match run_healthcheck(&config, &working_dir).await {
                Ok(()) => true,
                Err(HealthCheckError::Spawn(_)) => {
                    // Spawn errors (e.g. command not found) are fatal — stop the loop.
                    tracing::error!(name = %name, "healthcheck command cannot be spawned, stopping");
                    return;
                }
                _ => false,
            };
            if is_healthy && !was_healthy {
                let _ = logs_tx
                    .send(LogLine::new(
                        name.clone(),
                        LogStream::Stdout,
                        "healthcheck: passed".into(),
                        Utc::now(),
                    ))
                    .await;
            } else if !is_healthy && was_healthy {
                let _ = logs_tx
                    .send(LogLine::new(
                        name.clone(),
                        LogStream::Stdout,
                        "healthcheck: failed".into(),
                        Utc::now(),
                    ))
                    .await;
            }
            was_healthy = is_healthy;
        }
    });
}

// ---------------------------------------------------------------------------
// Linux parent-death signal (prctl)
// ---------------------------------------------------------------------------

/// Wrapper around `prctl(PR_SET_PDEATHSIG, SIGTERM)` — asks the kernel to
/// deliver SIGTERM to this process when its parent dies. Used in a `pre_exec`
/// hook so child processes are cleaned up even if the supervisor is SIGKILL'd.
#[cfg(target_os = "linux")]
mod linux_pdeathsig {
    use std::io;

    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGTERM: i32 = 15;

    unsafe extern "C" {
        fn prctl(option: i32, arg2: i64, arg3: i64, arg4: i64, arg5: i64) -> i32;
    }

    /// Call `prctl(PR_SET_PDEATHSIG, SIGTERM)`. Returns `Ok(())` on success,
    /// `Err(io::Error)` if the syscall fails.
    ///
    /// # Safety
    ///
    /// `prctl` is async-signal-safe and safe to call in the `pre_exec` context
    /// (single-threaded, no heap allocation). The raw FFI signature matches the
    /// Linux kernel ABI for `prctl`.
    pub fn apply() -> Result<(), io::Error> {
        let ret = unsafe { prctl(PR_SET_PDEATHSIG, SIGTERM as i64, 0, 0, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
