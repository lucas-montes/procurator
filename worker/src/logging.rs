//! Log output setup: JSON (default) or pretty, to stdout (default, captured
//! by journald under systemd) or to a file through `tracing-appender`.
//!
//! The `log` section is read from the same config file as everything else,
//! before the rest of the config is validated, so that validation errors are
//! already written in the chosen format. Shape:
//!
//! ```json
//! "log": { "format": "json", "file": { "path": "/var/log/cp.log", "rotation": "daily" } }
//! ```
//!
//! Convention lines (see `context/specs/log-events.md`) require the JSON
//! format to be parsed by the console.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, Layer as _, fmt::writer::BoxMakeWriter, layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    #[default]
    Json,
    Pretty,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Rotation {
    #[default]
    Never,
    Daily,
}

#[derive(Debug, Deserialize)]
struct FileTarget {
    path: PathBuf,
    #[serde(default)]
    rotation: Rotation,
}

#[derive(Debug, Default, Deserialize)]
struct LogConfig {
    #[serde(default)]
    format: Format,
    #[serde(default)]
    file: Option<FileTarget>,
}

#[derive(Debug, Default, Deserialize)]
struct Envelope {
    #[serde(default)]
    log: LogConfig,
}

/// Installs the global subscriber from the `log` section of `config_path`.
///
/// A missing or unreadable file falls back to JSON on stdout: the regular
/// config loading reports that error right after. `RUST_LOG` overrides
/// `default_filter`. Stdout is written synchronously (startup errors exit
/// right after logging); a file goes through a non-blocking writer whose
/// guard must stay alive for the whole process.
#[must_use]
pub fn init(config_path: &Path, default_filter: &str) -> Option<WorkerGuard> {
    let envelope: Envelope = std::fs::read(config_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let config = envelope.log;

    let (writer, guard) = match &config.file {
        None => (BoxMakeWriter::new(std::io::stdout), None),
        Some(target) => {
            let directory = target.path.parent().unwrap_or_else(|| Path::new("."));
            let file_name = target
                .path
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("worker.log"));
            if let Err(err) = std::fs::create_dir_all(directory) {
                eprintln!("cannot create log directory {}: {err}", directory.display());
            }
            let appender = match target.rotation {
                Rotation::Never => tracing_appender::rolling::never(directory, file_name),
                Rotation::Daily => tracing_appender::rolling::daily(directory, file_name),
            };
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (BoxMakeWriter::new(writer), Some(guard))
        }
    };

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let layer = match config.format {
        Format::Json => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(false)
            .with_span_list(false)
            .with_target(true)
            .with_ansi(false)
            .with_writer(writer)
            .boxed(),
        Format::Pretty => tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_ansi(config.file.is_none())
            .with_writer(writer)
            .boxed(),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .init();
    guard
}
