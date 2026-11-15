//! Message lines (`log.event = "msg"`, see `context/specs/log-events.md`).
//!
//! A [`Msg`] is created when a call starts and writes exactly one line when
//! dropped, so every early return of an RPC handler is covered. A message
//! never marked [`Msg::ok`] or [`Msg::rejected`] is written as
//! `error` (the handler failed through `?`).

use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    time::Instant,
};

/// One side of a message.
pub enum Endpoint {
    /// The service writing the line.
    Myself,
    /// A worker, by id.
    Worker(String),
    /// Any other RPC peer, by address when known.
    Client(Option<SocketAddr>),
}

impl Display for Endpoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Myself => f.write_str("self"),
            Self::Worker(id) => write!(f, "worker:{id}"),
            Self::Client(Some(addr)) => write!(f, "client:{addr}"),
            Self::Client(None) => f.write_str("client:unknown"),
        }
    }
}

enum Outcome {
    Unset,
    Ok,
    Rejected(String),
}

/// Pending message line, written on drop.
pub struct Msg {
    kind: &'static str,
    method: &'static str,
    from: Endpoint,
    to: Endpoint,
    started: Instant,
    outcome: Outcome,
    worker: Option<String>,
    generation: Option<u64>,
    count: Option<u64>,
    unchanged: Option<bool>,
}

impl Msg {
    /// Starts timing a message.
    pub fn start(kind: &'static str, method: &'static str, from: Endpoint, to: Endpoint) -> Self {
        Self {
            kind,
            method,
            from,
            to,
            started: Instant::now(),
            outcome: Outcome::Unset,
            worker: None,
            generation: None,
            count: None,
            unchanged: None,
        }
    }

    /// Records the worker the message is about.
    pub fn set_worker(&mut self, worker: &str) {
        self.worker = Some(worker.to_owned());
    }

    /// Records that the caller is `worker` (polls and heartbeats carry
    /// their worker id) instead of an anonymous client.
    pub fn set_caller_worker(&mut self, worker: &str) {
        self.from = Endpoint::Worker(worker.to_owned());
        self.worker = Some(worker.to_owned());
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = Some(generation);
    }

    pub fn set_count(&mut self, count: usize) {
        self.count = Some(u64::try_from(count).unwrap_or(u64::MAX));
    }

    pub fn set_unchanged(&mut self, unchanged: bool) {
        self.unchanged = Some(unchanged);
    }

    pub fn ok(&mut self) {
        self.outcome = Outcome::Ok;
    }

    pub fn rejected(&mut self, reason: impl Display) {
        self.outcome = Outcome::Rejected(reason.to_string());
    }
}

impl Drop for Msg {
    fn drop(&mut self) {
        let latency_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        let (outcome, reason) = match &self.outcome {
            Outcome::Ok => ("ok", None),
            Outcome::Rejected(reason) => ("rejected", Some(reason.as_str())),
            Outcome::Unset => ("error", Some("handler failed")),
        };
        tracing::info!(
            log.schema = 1_u64,
            log.event = "msg",
            msg.kind = self.kind,
            msg.method = self.method,
            msg.from = %self.from,
            msg.to = %self.to,
            msg.outcome = outcome,
            msg.reason = reason,
            latency_ms,
            worker = self.worker.as_deref(),
            generation = self.generation,
            count = self.count,
            unchanged = self.unchanged,
            "msg"
        );
    }
}
