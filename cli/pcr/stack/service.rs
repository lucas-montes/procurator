use core::fmt;
use std::{collections::HashMap, path::Path, path::PathBuf, process::Stdio};

use chrono::Utc;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead},
    process::Child,
    sync::mpsc::Sender,
};

use crate::stack::config;
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

    pub fn start(self, logger: Sender<LogLine>) -> Result<Service<Running>, Error> {
        let (prog, args) = parse_cmd(self.0.config.cmd())?;

        let mut child = tokio::process::Command::new(&prog)
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
}

// ---------------------------------------------------------------------------
// RunningManifest — set of running child processes
// ---------------------------------------------------------------------------

pub struct RunningManifest {
    services: HashMap<String, Service<Running>>,
}

impl RunningManifest {
    /// Kill all services in reverse dependency order.
    pub async fn kill_all(&mut self, order: &[String]) {
        for name in order.iter().rev() {
            if let Some(svc) = self.services.remove(name) {
                tracing::info!(name = %name, "shutting down service");
                let _ = svc.kill().await;
            }
        }
    }

    /// Remove and kill a single service by name.
    pub async fn remove(&mut self, name: &str) {
        if let Some(svc) = self.services.remove(name) {
            tracing::info!(name = %name, "stopping service");
            let _ = svc.kill().await;
        }
    }

    /// Insert a running service.
    pub fn insert(&mut self, name: String, svc: Service<Running>) {
        self.services.insert(name, svc);
    }
}

// ---------------------------------------------------------------------------
// Supervisor — event loop
// ---------------------------------------------------------------------------

pub struct Supervisor {
    manifest: ServiceManifest,
    logs_tx: Sender<LogLine>,
    watcher_rx: tokio::sync::mpsc::Receiver<WatchEvent>,
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
        }
    }

    /// Start all services from the initial manifest, then enter the event loop.
    /// On Ctrl-C, clean shutdown. On watcher signal, diff and restart.
    pub async fn run(mut self) -> Result<(), Error> {
        let mut running = self.manifest.clone().start_all(self.logs_tx.clone())?;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received SIGINT, shutting down");
                    break;
                }
                Some(event) = self.watcher_rx.recv() => {
                    match event {
                        WatchEvent::ConfigChanged(new_manifest) => {
                            self.apply_manifest(&new_manifest, &mut running).await;
                            self.manifest = new_manifest;
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
            }
        }

        running.kill_all(self.manifest.order()).await;
        Ok(())
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
