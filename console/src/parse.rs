//! Parses service log lines per `context/specs/log-events.md`. Strings borrow
//! from the line whenever they contain no escapes.

use std::borrow::Cow;

use serde::Deserialize;

/// Every field the convention defines; anything else in the line is ignored.
#[derive(Debug, Default, Deserialize)]
pub struct Line<'a> {
    #[serde(borrow, default)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    level: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    target: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    message: Option<Cow<'a, str>>,
    #[serde(rename = "log.schema", default)]
    schema: Option<u64>,
    #[serde(rename = "log.event", borrow, default)]
    event: Option<Cow<'a, str>>,

    #[serde(rename = "msg.kind", borrow, default)]
    kind: Option<Cow<'a, str>>,
    #[serde(rename = "msg.method", borrow, default)]
    method: Option<Cow<'a, str>>,
    #[serde(rename = "msg.from", borrow, default)]
    from: Option<Cow<'a, str>>,
    #[serde(rename = "msg.to", borrow, default)]
    to: Option<Cow<'a, str>>,
    #[serde(rename = "msg.outcome", borrow, default)]
    outcome: Option<Cow<'a, str>>,
    #[serde(rename = "msg.reason", borrow, default)]
    reason: Option<Cow<'a, str>>,
    #[serde(default)]
    latency_ms: Option<f64>,

    #[serde(borrow, default)]
    worker: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    vm: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    unit: Option<Cow<'a, str>>,
    #[serde(default)]
    generation: Option<u64>,
    #[serde(default)]
    count: Option<u64>,
    #[serde(default)]
    unchanged: Option<bool>,

    #[serde(borrow, default)]
    scope: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    commit: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    vms: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    status: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    ip: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    change: Option<Cow<'a, str>>,
    #[serde(default)]
    uptime_s: Option<u64>,
    #[serde(default)]
    cpu_ms: Option<u64>,
    #[serde(default)]
    rss_bytes: Option<u64>,
    #[serde(default)]
    net_rx: Option<u64>,
    #[serde(default)]
    net_tx: Option<u64>,
    #[serde(default)]
    disk_read: Option<u64>,
    #[serde(default)]
    disk_write: Option<u64>,
    #[serde(rename = "host.mem_total", default)]
    mem_total: Option<u64>,
    #[serde(rename = "host.mem_available", default)]
    mem_available: Option<u64>,
    #[serde(rename = "host.load1", default)]
    load1: Option<f64>,
    #[serde(rename = "host.cpus", default)]
    cpus: Option<u64>,
}

/// What a line is, after validating the fields its kind requires.
pub enum Parsed<'a> {
    /// A `msg` line.
    Message(Line<'a>),
    /// A `state` line with a known scope.
    State(Line<'a>),
    /// A `change` line.
    Change(Line<'a>),
    /// Any other JSON log line.
    Plain(Line<'a>),
}

/// Why a line could not be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Unparsed {
    /// Not a JSON object (e.g. the `pretty` log format).
    NotJson,
    /// A convention line with an unknown `log.schema`.
    Schema,
    /// A convention line missing a field its kind requires.
    Missing(&'static str),
}

/// Parses one line.
///
/// # Errors
///
/// Returns [`Unparsed`] when the line can't be used.
pub fn parse(text: &str) -> Result<Parsed<'_>, Unparsed> {
    let line: Line<'_> = match serde_json::from_str(text) {
        Ok(line) => line,
        Err(_) => return parse_loose(text),
    };
    let Some(event) = line.event.as_deref() else {
        return Ok(Parsed::Plain(line));
    };
    if line.schema != Some(1) {
        return Err(Unparsed::Schema);
    }
    match event {
        "msg" => {
            require(line.kind.is_some(), "msg.kind")?;
            require(line.method.is_some(), "msg.method")?;
            require(line.from.is_some(), "msg.from")?;
            require(line.to.is_some(), "msg.to")?;
            require(line.outcome.is_some(), "msg.outcome")?;
            Ok(Parsed::Message(line))
        }
        "state" => {
            match line.scope.as_deref() {
                Some("cluster") => {}
                Some("desired" | "worker") => require(line.worker.is_some(), "worker")?,
                Some("vm") => {
                    require(line.worker.is_some(), "worker")?;
                    require(line.vm.is_some(), "vm")?;
                }
                _ => return Err(Unparsed::Missing("scope")),
            }
            Ok(Parsed::State(line))
        }
        "change" => {
            require(line.change.is_some(), "change")?;
            Ok(Parsed::Change(line))
        }
        _ => Err(Unparsed::Missing("log.event")),
    }
}

/// Plain lines may carry any field with any type (a `vms` count, say), so
/// when the strict parse fails only the fields plain lines need are kept.
#[derive(Deserialize)]
struct Loose<'a> {
    #[serde(borrow, default)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    level: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    target: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    message: Option<Cow<'a, str>>,
    #[serde(default)]
    worker: Option<serde_json::Value>,
    #[serde(default)]
    vm: Option<serde_json::Value>,
    #[serde(rename = "log.event", default)]
    event: Option<serde::de::IgnoredAny>,
}

fn parse_loose(text: &str) -> Result<Parsed<'_>, Unparsed> {
    let loose: Loose<'_> = serde_json::from_str(text).map_err(|_| Unparsed::NotJson)?;
    if loose.event.is_some() {
        // A convention line whose fields have the wrong types.
        return Err(Unparsed::Missing("well-typed fields"));
    }
    let text_of = |value: Option<serde_json::Value>| match value {
        Some(serde_json::Value::String(text)) => Some(Cow::Owned(text)),
        _ => None,
    };
    Ok(Parsed::Plain(Line {
        timestamp: loose.timestamp,
        level: loose.level,
        target: loose.target,
        message: loose.message,
        worker: text_of(loose.worker),
        vm: text_of(loose.vm),
        ..Line::default()
    }))
}

fn require(present: bool, field: &'static str) -> Result<(), Unparsed> {
    if present {
        Ok(())
    } else {
        Err(Unparsed::Missing(field))
    }
}

impl Line<'_> {
    pub fn timestamp(&self) -> Option<&str> {
        self.timestamp.as_deref()
    }
    pub fn level(&self) -> Option<&str> {
        self.level.as_deref()
    }
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }
    pub fn method(&self) -> Option<&str> {
        self.method.as_deref()
    }
    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }
    pub fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    pub fn latency_ms(&self) -> Option<f64> {
        self.latency_ms
    }
    pub fn worker(&self) -> Option<&str> {
        self.worker.as_deref()
    }
    pub fn vm(&self) -> Option<&str> {
        self.vm.as_deref()
    }
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
    pub fn generation(&self) -> Option<u64> {
        self.generation
    }
    pub fn count(&self) -> Option<u64> {
        self.count
    }
    pub fn unchanged(&self) -> Option<bool> {
        self.unchanged
    }
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
    pub fn commit(&self) -> Option<&str> {
        self.commit.as_deref()
    }
    pub fn vms(&self) -> Option<&str> {
        self.vms.as_deref()
    }
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
    pub fn ip(&self) -> Option<&str> {
        self.ip.as_deref()
    }
    pub fn change(&self) -> Option<&str> {
        self.change.as_deref()
    }
    pub fn uptime_s(&self) -> Option<u64> {
        self.uptime_s
    }
    pub fn cpu_ms(&self) -> Option<u64> {
        self.cpu_ms
    }
    pub fn rss_bytes(&self) -> Option<u64> {
        self.rss_bytes
    }
    pub fn net_rx(&self) -> Option<u64> {
        self.net_rx
    }
    pub fn net_tx(&self) -> Option<u64> {
        self.net_tx
    }
    pub fn disk_read(&self) -> Option<u64> {
        self.disk_read
    }
    pub fn disk_write(&self) -> Option<u64> {
        self.disk_write
    }
    pub fn mem_total(&self) -> Option<u64> {
        self.mem_total
    }
    pub fn mem_available(&self) -> Option<u64> {
        self.mem_available
    }
    pub fn load1(&self) -> Option<f64> {
        self.load1
    }
    pub fn cpus(&self) -> Option<u64> {
        self.cpus
    }
}

#[cfg(test)]
mod tests {
    use super::{Parsed, Unparsed, parse};

    #[test]
    fn classifies_lines() {
        let msg = r#"{"timestamp":"2026-11-15T10:00:00Z","log.schema":1,"log.event":"msg","msg.kind":"poll","msg.method":"getAssignment","msg.from":"worker:w1","msg.to":"self","msg.outcome":"ok","latency_ms":0.2,"unchanged":true}"#;
        assert!(matches!(parse(msg), Ok(Parsed::Message(line)) if line.unchanged() == Some(true)));

        let vm = r#"{"log.schema":1,"log.event":"state","scope":"vm","worker":"w1","vm":"web","status":"running"}"#;
        assert!(matches!(parse(vm), Ok(Parsed::State(_))));

        let plain = r#"{"level":"INFO","message":"hello","vm":"web"}"#;
        assert!(matches!(parse(plain), Ok(Parsed::Plain(line)) if line.vm() == Some("web")));

        // Arbitrary fields on plain lines may clash with convention names.
        let clash = r#"{"level":"DEBUG","message":"Heartbeat pushed","worker":"w1","vms":0}"#;
        assert!(matches!(parse(clash), Ok(Parsed::Plain(line)) if line.worker() == Some("w1")));
    }

    #[test]
    fn reports_why_a_line_is_unusable() {
        assert_eq!(
            parse("2026 INFO pretty line").err(),
            Some(Unparsed::NotJson)
        );
        assert_eq!(
            parse(r#"{"log.schema":2,"log.event":"msg"}"#).err(),
            Some(Unparsed::Schema)
        );
        assert_eq!(
            parse(r#"{"log.schema":1,"log.event":"state","scope":"vm","worker":"w1"}"#).err(),
            Some(Unparsed::Missing("vm"))
        );
    }
}
