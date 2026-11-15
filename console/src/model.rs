//! The live model: the single owner of everything the console knows, built
//! only from parsed log lines.
//!
//! - **State lines** update machines, VMs and the published generation.
//!   A snapshot is serialized at most every 500 ms when something changed,
//!   and every 2 s regardless (ages and health move with time).
//! - **Message lines** are fanned out live; **change and plain lines** are
//!   fanned out as logs. Lines older than the console start (read back from
//!   history to rebuild state) are never streamed: the view is live-only.

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::{
    sync::{broadcast, mpsc, watch},
    time::{Duration, MissedTickBehavior, interval},
};

use crate::{
    config::SourceConfig,
    parse::{Line, Parsed, parse},
    sources::SourceEvent,
};

/// How many unparsed lines each source keeps for the Sources page.
const RECENT_UNPARSED: usize = 5;
/// Unparsed lines are cut to this many bytes.
const UNPARSED_PREVIEW: usize = 300;
/// Lines stamped this long before the console started still count as live
/// (clock skew between hosts).
const LIVE_SLACK_MS: i64 = 2000;

#[derive(Serialize)]
struct SourceState {
    name: String,
    endpoint: String,
    kind: String,
    connected: bool,
    error: Option<String>,
    lines: u64,
    unparsed: u64,
    #[serde(skip)]
    last_line: Option<DateTime<Utc>>,
    recent_unparsed: VecDeque<String>,
}

#[derive(Default, Serialize)]
struct Host {
    mem_total: u64,
    mem_available: u64,
    load1: f64,
    cpus: u64,
}

#[derive(Serialize)]
struct VmObserved {
    status: String,
    ip: String,
    unit: Option<String>,
    uptime_s: u64,
    cpu_ms: u64,
    /// CPU use of the VMM process between the last two samples, in percent
    /// of one core.
    cpu_pct: Option<f64>,
    rss_bytes: u64,
    net_rx: u64,
    net_tx: u64,
    disk_read: u64,
    disk_write: u64,
    #[serde(skip)]
    sampled_at: DateTime<Utc>,
    /// Worker state line (tick) this VM was last reported in.
    #[serde(skip)]
    tick: u64,
}

#[derive(Default)]
struct WorkerState {
    generation: u64,
    count: u64,
    last_seen: Option<DateTime<Utc>>,
    host: Host,
    tick: u64,
    vms: BTreeMap<String, VmObserved>,
    desired: Vec<String>,
}

#[derive(Default)]
struct Cluster {
    generation: u64,
    commit: String,
    last_seen: Option<DateTime<Utc>>,
}

/// Receiving ends handed to the HTTP layer.
pub struct Channels {
    snapshot: watch::Receiver<String>,
    messages: broadcast::Sender<String>,
    logs: broadcast::Sender<String>,
}

impl Channels {
    pub fn into_parts(
        self,
    ) -> (
        watch::Receiver<String>,
        broadcast::Sender<String>,
        broadcast::Sender<String>,
    ) {
        (self.snapshot, self.messages, self.logs)
    }
}

pub struct Model {
    sources: Vec<SourceState>,
    cluster: Cluster,
    workers: BTreeMap<String, WorkerState>,
    started: DateTime<Utc>,
    stale_after_ms: i64,
    messages: broadcast::Sender<String>,
    logs: broadcast::Sender<String>,
    snapshot: watch::Sender<String>,
    dirty: bool,
}

impl Model {
    pub fn new(sources: &[SourceConfig], stale_after_ms: u64) -> (Self, Channels) {
        let (messages, _) = broadcast::channel(1024);
        let (logs, _) = broadcast::channel(1024);
        let (snapshot, snapshot_rx) = watch::channel(String::from("{}"));
        let mut workers = BTreeMap::new();
        for source in sources {
            if let Some(id) = source.endpoint().strip_prefix("worker:") {
                workers.insert(id.to_owned(), WorkerState::default());
            }
        }
        let model = Self {
            sources: sources
                .iter()
                .map(|source| SourceState {
                    name: source.name().to_owned(),
                    endpoint: source.endpoint().to_owned(),
                    kind: source.kind().to_string(),
                    connected: false,
                    error: None,
                    lines: 0,
                    unparsed: 0,
                    last_line: None,
                    recent_unparsed: VecDeque::new(),
                })
                .collect(),
            cluster: Cluster::default(),
            workers,
            started: Utc::now(),
            stale_after_ms: i64::try_from(stale_after_ms).unwrap_or(i64::MAX),
            messages: messages.clone(),
            logs: logs.clone(),
            snapshot,
            dirty: true,
        };
        let channels = Channels {
            snapshot: snapshot_rx,
            messages,
            logs,
        };
        (model, channels)
    }

    /// Consumes source events until every source is gone.
    pub async fn run(mut self, mut events: mpsc::Receiver<SourceEvent>) {
        let mut ticker = interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut ticks: u32 = 0;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    None => break,
                    Some(SourceEvent::Line { source, text }) => self.handle_line(source, &text, Utc::now()),
                    Some(SourceEvent::Status { source, error }) => self.set_status(source, error),
                },
                _ = ticker.tick() => {
                    ticks = ticks.wrapping_add(1);
                    if self.dirty || ticks.is_multiple_of(4) {
                        self.publish(Utc::now());
                    }
                }
            }
        }
    }

    fn set_status(&mut self, source: usize, error: Option<String>) {
        if let Some(state) = self.sources.get_mut(source) {
            state.connected = error.is_none();
            state.error = error;
            self.dirty = true;
        }
    }

    fn handle_line(&mut self, source: usize, text: &str, now: DateTime<Utc>) {
        let Some(state) = self.sources.get_mut(source) else {
            return;
        };
        state.lines = state.lines.saturating_add(1);
        state.last_line = Some(now);
        let Ok(parsed) = parse(text) else {
            state.unparsed = state.unparsed.saturating_add(1);
            if state.recent_unparsed.len() == RECENT_UNPARSED {
                state.recent_unparsed.pop_front();
            }
            state.recent_unparsed.push_back(preview(text).to_owned());
            return;
        };
        let is_fresh = is_live(&parsed, now, self.started);
        let origin = self.sources[source].endpoint.as_str();
        match parsed {
            Parsed::Message(line) => {
                if is_fresh {
                    send(&self.messages, &message_view(&line, origin));
                }
            }
            Parsed::Change(line) => {
                if line.change() == Some("vm_deleted")
                    && let (Some(id), Some(vm)) = (origin.strip_prefix("worker:"), line.vm())
                    && let Some(worker) = self.workers.get_mut(id)
                {
                    worker.vms.remove(vm);
                    self.dirty = true;
                }
                if is_fresh {
                    send(&self.logs, &log_view(&line, origin));
                }
            }
            Parsed::Plain(line) => {
                if is_fresh {
                    send(&self.logs, &log_view(&line, origin));
                }
            }
            Parsed::State(line) => {
                let at = timestamp(&line).unwrap_or(now);
                self.apply_state(&line, at);
                self.dirty = true;
            }
        }
    }

    fn apply_state(&mut self, line: &Line<'_>, at: DateTime<Utc>) {
        match line.scope() {
            Some("cluster") => {
                self.cluster.generation = line.generation().unwrap_or(0);
                line.commit()
                    .unwrap_or_default()
                    .clone_into(&mut self.cluster.commit);
                self.cluster.last_seen = Some(at);
            }
            Some("desired") => {
                let Some(id) = line.worker() else { return };
                let worker = self.workers.entry(id.to_owned()).or_default();
                worker.desired.clear();
                worker.desired.extend(
                    line.vms()
                        .unwrap_or_default()
                        .split(',')
                        .filter(|key| !key.is_empty())
                        .map(str::to_owned),
                );
            }
            Some("worker") => {
                let Some(id) = line.worker() else { return };
                let worker = self.workers.entry(id.to_owned()).or_default();
                worker.generation = line.generation().unwrap_or(0);
                worker.count = line.count().unwrap_or(0);
                worker.host = Host {
                    mem_total: line.mem_total().unwrap_or(0),
                    mem_available: line.mem_available().unwrap_or(0),
                    load1: line.load1().unwrap_or(0.0),
                    cpus: line.cpus().unwrap_or(0),
                };
                if worker.last_seen.is_none_or(|seen| at > seen) {
                    worker.last_seen = Some(at);
                }
                // VM lines follow their worker line; a VM missing from the
                // previous tick is gone.
                worker.tick = worker.tick.saturating_add(1);
                let tick = worker.tick;
                worker.vms.retain(|_, vm| vm.tick.saturating_add(1) >= tick);
            }
            Some("vm") => {
                let (Some(id), Some(key)) = (line.worker(), line.vm()) else {
                    return;
                };
                let worker = self.workers.entry(id.to_owned()).or_default();
                let tick = worker.tick;
                let cpu_ms = line.cpu_ms().unwrap_or(0);
                let cpu_pct = worker.vms.get(key).and_then(|previous| {
                    let elapsed = (at - previous.sampled_at).num_milliseconds();
                    let used = cpu_ms.checked_sub(previous.cpu_ms)?;
                    (elapsed > 0).then(|| {
                        #[allow(clippy::cast_precision_loss)]
                        let pct = used as f64 * 100.0 / elapsed as f64;
                        pct
                    })
                });
                worker.vms.insert(
                    key.to_owned(),
                    VmObserved {
                        status: line.status().unwrap_or("running").to_owned(),
                        ip: line.ip().unwrap_or_default().to_owned(),
                        unit: line.unit().map(str::to_owned),
                        uptime_s: line.uptime_s().unwrap_or(0),
                        cpu_ms,
                        cpu_pct,
                        rss_bytes: line.rss_bytes().unwrap_or(0),
                        net_rx: line.net_rx().unwrap_or(0),
                        net_tx: line.net_tx().unwrap_or(0),
                        disk_read: line.disk_read().unwrap_or(0),
                        disk_write: line.disk_write().unwrap_or(0),
                        sampled_at: at,
                        tick,
                    },
                );
            }
            _ => {}
        }
    }

    fn publish(&mut self, now: DateTime<Utc>) {
        let view = self.view(now);
        match serde_json::to_string(&view) {
            Ok(json) => {
                self.snapshot.send_replace(json);
            }
            Err(err) => tracing::error!(%err, "cannot serialize snapshot"),
        }
        self.dirty = false;
    }

    fn view(&self, now: DateTime<Utc>) -> SnapshotView<'_> {
        let cluster_known = self.cluster.last_seen.is_some();
        let workers = self
            .workers
            .iter()
            .map(|(id, worker)| {
                let age_s = worker.last_seen.map(|seen| age_seconds(seen, now));
                let healthy = worker
                    .last_seen
                    .is_some_and(|seen| (now - seen).num_milliseconds() <= self.stale_after_ms);
                let mut vms: Vec<VmView<'_>> = Vec::with_capacity(worker.vms.len());
                for (key, observed) in &worker.vms {
                    let desired = worker.desired.iter().any(|desired| desired == key);
                    let state = if cluster_known && !desired {
                        "orphan"
                    } else {
                        observed.status.as_str()
                    };
                    vms.push(VmView {
                        key,
                        state,
                        desired,
                        observed: Some(observed),
                    });
                }
                for key in &worker.desired {
                    if !worker.vms.contains_key(key) {
                        vms.push(VmView {
                            key,
                            state: "missing",
                            desired: true,
                            observed: None,
                        });
                    }
                }
                vms.sort_by(|left, right| left.key.cmp(right.key));
                let up_to_date = cluster_known
                    && worker.generation == self.cluster.generation
                    && vms.iter().all(|vm| vm.state == "running");
                WorkerView {
                    id,
                    healthy,
                    age_s,
                    generation: worker.generation,
                    count: worker.count,
                    up_to_date,
                    host: &worker.host,
                    vms,
                }
            })
            .collect();
        SnapshotView {
            now: now.to_rfc3339(),
            cluster: ClusterView {
                known: cluster_known,
                generation: self.cluster.generation,
                commit: &self.cluster.commit,
                age_s: self.cluster.last_seen.map(|seen| age_seconds(seen, now)),
            },
            workers,
            sources: self
                .sources
                .iter()
                .map(|source| SourceView {
                    state: source,
                    last_line_age_s: source.last_line.map(|seen| age_seconds(seen, now)),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct SnapshotView<'a> {
    now: String,
    cluster: ClusterView<'a>,
    workers: Vec<WorkerView<'a>>,
    sources: Vec<SourceView<'a>>,
}

#[derive(Serialize)]
struct ClusterView<'a> {
    known: bool,
    generation: u64,
    commit: &'a str,
    age_s: Option<f64>,
}

#[derive(Serialize)]
struct WorkerView<'a> {
    id: &'a str,
    healthy: bool,
    age_s: Option<f64>,
    generation: u64,
    count: u64,
    up_to_date: bool,
    host: &'a Host,
    vms: Vec<VmView<'a>>,
}

#[derive(Serialize)]
struct VmView<'a> {
    key: &'a str,
    /// `running`, `failed`, `missing` (desired, not running yet) or
    /// `orphan` (running, not desired).
    state: &'a str,
    desired: bool,
    #[serde(flatten)]
    observed: Option<&'a VmObserved>,
}

#[derive(Serialize)]
struct SourceView<'a> {
    #[serde(flatten)]
    state: &'a SourceState,
    last_line_age_s: Option<f64>,
}

#[derive(Serialize)]
struct MessageView<'a> {
    at: Option<&'a str>,
    origin: &'a str,
    kind: &'a str,
    method: &'a str,
    from: &'a str,
    to: &'a str,
    outcome: &'a str,
    reason: Option<&'a str>,
    latency_ms: Option<f64>,
    worker: Option<&'a str>,
    vm: Option<&'a str>,
    generation: Option<u64>,
    count: Option<u64>,
    unchanged: Option<bool>,
}

#[derive(Serialize)]
struct LogView<'a> {
    at: Option<&'a str>,
    origin: &'a str,
    level: Option<&'a str>,
    target: Option<&'a str>,
    message: Option<&'a str>,
    change: Option<&'a str>,
    worker: Option<&'a str>,
    vm: Option<&'a str>,
}

/// Resolves `self` to the endpoint of the service that wrote the line.
fn resolve<'a>(endpoint: &'a str, origin: &'a str) -> &'a str {
    if endpoint == "self" { origin } else { endpoint }
}

fn message_view<'a>(line: &'a Line<'a>, origin: &'a str) -> MessageView<'a> {
    MessageView {
        at: line.timestamp(),
        origin,
        kind: line.kind().unwrap_or_default(),
        method: line.method().unwrap_or_default(),
        from: resolve(line.from().unwrap_or_default(), origin),
        to: resolve(line.to().unwrap_or_default(), origin),
        outcome: line.outcome().unwrap_or_default(),
        reason: line.reason(),
        latency_ms: line.latency_ms(),
        worker: line.worker(),
        vm: line.vm(),
        generation: line.generation(),
        count: line.count(),
        unchanged: line.unchanged(),
    }
}

fn log_view<'a>(line: &'a Line<'a>, origin: &'a str) -> LogView<'a> {
    LogView {
        at: line.timestamp(),
        origin,
        level: line.level(),
        target: line.target(),
        message: line.message(),
        change: line.change(),
        worker: line.worker(),
        vm: line.vm(),
    }
}

fn send(channel: &broadcast::Sender<String>, view: &impl Serialize) {
    // No receivers (no browser open) is fine.
    if channel.receiver_count() > 0
        && let Ok(json) = serde_json::to_string(view)
    {
        let _ = channel.send(json);
    }
}

/// Whether a line happened after the console started (not history read
/// back to rebuild state).
fn is_live(parsed: &Parsed<'_>, now: DateTime<Utc>, started: DateTime<Utc>) -> bool {
    let line = match parsed {
        Parsed::Message(line)
        | Parsed::State(line)
        | Parsed::Change(line)
        | Parsed::Plain(line) => line,
    };
    let at = timestamp(line).unwrap_or(now);
    (at - started).num_milliseconds() >= -LIVE_SLACK_MS
}

fn timestamp(line: &Line<'_>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(line.timestamp()?)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn age_seconds(seen: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let seconds = (now - seen).num_milliseconds() as f64 / 1000.0;
    seconds.max(0.0)
}

/// The first `UNPARSED_PREVIEW` bytes of `text`, cut on a char boundary.
fn preview(text: &str) -> &str {
    if text.len() <= UNPARSED_PREVIEW {
        return text;
    }
    let mut end = UNPARSED_PREVIEW;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::Model;
    use crate::config::Config;

    fn model() -> Model {
        let config: Config = serde_json::from_str(
            r#"{"sources": [
                {"name": "cp", "service": "control_plane", "kind": {"file": {"path": "cp.log"}}},
                {"name": "w1", "service": "worker:w1", "kind": {"file": {"path": "w1.log"}}}
            ]}"#,
        )
        .expect("config");
        Model::new(config.sources(), 15_000).0
    }

    fn line(at: chrono::DateTime<Utc>, body: &str) -> String {
        format!(
            r#"{{"timestamp":"{}","log.schema":1,{body}}}"#,
            at.to_rfc3339()
        )
    }

    fn snapshot(model: &mut Model, now: chrono::DateTime<Utc>) -> serde_json::Value {
        model.publish(now);
        serde_json::from_str(&model.snapshot.borrow()).expect("snapshot json")
    }

    #[test]
    fn joins_desired_and_observed_vms() {
        let mut model = model();
        let now = Utc::now();
        let cp = [
            r#""log.event":"state","scope":"cluster","generation":3,"commit":"abc","count":1"#,
            r#""log.event":"state","scope":"desired","worker":"w1","vms":"db,web","count":2"#,
        ];
        for body in cp {
            model.handle_line(0, &line(now, body), now);
        }
        let worker = [
            r#""log.event":"state","scope":"worker","worker":"w1","generation":3,"count":2"#,
            r#""log.event":"state","scope":"vm","worker":"w1","vm":"web","status":"running","ip":"10.0.0.2","cpu_ms":1000"#,
            r#""log.event":"state","scope":"vm","worker":"w1","vm":"stray","status":"running","cpu_ms":0"#,
        ];
        for body in worker {
            model.handle_line(1, &line(now, body), now);
        }
        let view = snapshot(&mut model, now);
        let vms = &view["workers"][0]["vms"];
        let states: Vec<(&str, &str)> = vms
            .as_array()
            .expect("vms")
            .iter()
            .map(|vm| {
                (
                    vm["key"].as_str().unwrap_or(""),
                    vm["state"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            states,
            vec![("db", "missing"), ("stray", "orphan"), ("web", "running")]
        );
        assert_eq!(view["workers"][0]["healthy"], true);
        assert_eq!(view["workers"][0]["up_to_date"], false);
        assert_eq!(view["cluster"]["generation"], 3);
    }

    #[test]
    fn derives_cpu_rate_prunes_gone_vms_and_ages_out_health() {
        let mut model = model();
        let start = Utc::now();
        let later = start + Duration::seconds(5);
        let tick = |model: &mut Model, at, vms: &[(&str, u64)]| {
            let worker = r#""log.event":"state","scope":"worker","worker":"w1","generation":1"#;
            model.handle_line(1, &line(at, worker), at);
            for (vm, cpu) in vms {
                let body = format!(
                    r#""log.event":"state","scope":"vm","worker":"w1","vm":"{vm}","status":"running","cpu_ms":{cpu}"#
                );
                model.handle_line(1, &line(at, &body), at);
            }
        };
        tick(&mut model, start, &[("a", 0), ("b", 0)]);
        tick(&mut model, later, &[("a", 2500)]);
        let view = snapshot(&mut model, later);
        assert_eq!(view["workers"][0]["vms"][0]["cpu_pct"], 50.0);
        assert_eq!(view["workers"][0]["vms"].as_array().map(Vec::len), Some(2));

        // `b` was absent from the previous tick: pruned on the next one.
        tick(&mut model, later + Duration::seconds(5), &[("a", 2500)]);
        let view = snapshot(&mut model, later + Duration::seconds(5));
        assert_eq!(view["workers"][0]["vms"].as_array().map(Vec::len), Some(1));

        let view = snapshot(&mut model, later + Duration::seconds(60));
        assert_eq!(view["workers"][0]["healthy"], false);
    }

    #[test]
    fn counts_unparsed_lines_per_source() {
        let mut model = model();
        let now = Utc::now();
        model.handle_line(1, "not json at all", now);
        model.handle_line(1, r#"{"log.schema":9,"log.event":"msg"}"#, now);
        let view = snapshot(&mut model, now);
        assert_eq!(view["sources"][1]["unparsed"], 2);
        assert_eq!(view["sources"][1]["lines"], 2);
        assert_eq!(view["sources"][1]["recent_unparsed"][0], "not json at all");
    }
}
