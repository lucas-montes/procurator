//! Message lines (`log.event = "msg"`, see `context/specs/log-events.md`).
//!
//! A [`Msg`] is created when a call starts and writes exactly one line when
//! dropped. A message never marked [`Msg::ok`] or [`Msg::error`] is
//! written as `error`.

use std::{
    fmt::{Display, Formatter},
    net::SocketAddr,
    time::Instant,
};

/// One side of a message.
pub enum Endpoint<'a> {
    /// The service writing the line.
    Myself,
    /// The control plane.
    ControlPlane,
    /// Any other RPC peer, by address when known.
    Client(Option<SocketAddr>),
    /// The Cloud Hypervisor process of a VM.
    Vmm(&'a str),
}

impl Display for Endpoint<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Myself => f.write_str("self"),
            Self::ControlPlane => f.write_str("cp"),
            Self::Client(Some(addr)) => write!(f, "client:{addr}"),
            Self::Client(None) => f.write_str("client:unknown"),
            Self::Vmm(vm) => write!(f, "vmm:{vm}"),
        }
    }
}

enum Outcome {
    Unset,
    Ok,
    Error(String),
}

/// Pending message line, written on drop.
pub struct Msg<'a> {
    kind: &'static str,
    method: &'a str,
    from: Endpoint<'a>,
    to: Endpoint<'a>,
    started: Instant,
    outcome: Outcome,
    worker: Option<&'a str>,
    vm: Option<&'a str>,
    silent: bool,
}

impl<'a> Msg<'a> {
    /// Starts timing a message.
    pub fn start(
        kind: &'static str,
        method: &'a str,
        from: Endpoint<'a>,
        to: Endpoint<'a>,
    ) -> Self {
        Self {
            kind,
            method,
            from,
            to,
            started: Instant::now(),
            outcome: Outcome::Unset,
            worker: None,
            vm: None,
            silent: false,
        }
    }

    #[must_use]
    pub fn worker(mut self, worker: &'a str) -> Self {
        self.worker = Some(worker);
        self
    }

    #[must_use]
    pub fn vm(mut self, vm: &'a str) -> Self {
        self.vm = Some(vm);
        self
    }

    pub fn ok(&mut self) {
        self.outcome = Outcome::Ok;
    }

    pub fn error(&mut self, reason: impl Display) {
        self.outcome = Outcome::Error(reason.to_string());
    }

    /// Drops the message without writing a line (the other side logs it).
    pub fn discard(mut self) {
        self.silent = true;
    }
}

impl Drop for Msg<'_> {
    fn drop(&mut self) {
        if self.silent {
            return;
        }
        let latency_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        let (outcome, reason) = match &self.outcome {
            Outcome::Ok => ("ok", None),
            Outcome::Error(reason) => ("error", Some(reason.as_str())),
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
            worker = self.worker,
            vm = self.vm,
            "msg"
        );
    }
}

/// Runs an RPC handler body and writes its message line with the outcome.
pub fn logged<F>(
    method: &'static str,
    peer: Option<SocketAddr>,
    body: F,
) -> capnp::capability::Promise<(), capnp::Error>
where
    F: Future<Output = Result<(), capnp::Error>> + 'static,
{
    capnp::capability::Promise::from_future(async move {
        let mut msg = Msg::start("rpc", method, Endpoint::Client(peer), Endpoint::Myself);
        let result = body.await;
        match &result {
            Ok(()) => msg.ok(),
            Err(err) => msg.error(err),
        }
        result
    })
}
