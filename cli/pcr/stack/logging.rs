use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::marker::PhantomData;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

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
pub fn color_for(name: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = hasher.finish() as usize % COLORS.len();
    COLORS[idx]
}

/// A colored `[service_name]` prefix that implements `Display`.
pub struct ColoredPrefix<'a> {
    name: &'a str,
    color: &'static str,
}

impl<'a> ColoredPrefix<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            color: color_for(name),
        }
    }
}

impl fmt::Display for ColoredPrefix<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{color}[{name}]{reset}",
            color = self.color,
            name = self.name,
            reset = RESET
        )
    }
}

// ---------------------------------------------------------------------------
// LogLine
// ---------------------------------------------------------------------------

/// Marker for terminal (colored) display.
pub struct Terminal;

/// Marker for file (plain-text) display.
pub struct File;

/// Whether the log line came from stdout or stderr.
#[derive(Debug, Clone)]
pub enum LogStream {
    Stdout,
    Stderr,
}

impl fmt::Display for LogStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogStream::Stdout => write!(f, "STDOUT"),
            LogStream::Stderr => write!(f, "STDERR"),
        }
    }
}

/// A single log line from a service.
///
/// The type parameter `T` selects the `Display` implementation:
/// - [`Terminal`] — coloured prefix via `color_for`
/// - [`File`] — plain timestamped format
#[derive(Debug, Clone)]
pub struct LogLine<T = Terminal> {
    pub service: String,
    pub stream: LogStream,
    pub text: String,
    pub timestamp: DateTime<Utc>,
    _marker: PhantomData<T>,
}

impl LogLine {
    pub fn new(service: String, stream: LogStream, text: String, timestamp: DateTime<Utc>) -> Self {
        Self {
            service,
            stream,
            text,
            timestamp,
            _marker: PhantomData,
        }
    }
}

// Terminal display — coloured prefix, no timestamp.
impl fmt::Display for LogLine<Terminal> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = ColoredPrefix::new(&self.service);
        match self.stream {
            LogStream::Stdout => write!(f, "{} {}", prefix, self.text),
            LogStream::Stderr => write!(f, "{} ERR {}", prefix, self.text),
        }
    }
}

// File display — plain, timestamped.
impl fmt::Display for LogLine<File> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] [{}] {} {}",
            self.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            self.service,
            self.stream,
            self.text,
        )
    }
}

// ---------------------------------------------------------------------------
// LogWriter trait
// ---------------------------------------------------------------------------

/// Sync trait for writing log lines. Runs inside a dedicated task,
/// so blocking I/O is acceptable.
pub trait LogWriter: Send {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), String>;
    fn flush(&mut self) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// TerminalWriter
// ---------------------------------------------------------------------------

/// Writes log lines to terminal with coloured service prefixes.
#[derive(Debug, Default)]
pub struct TerminalWriter;

impl LogWriter for TerminalWriter {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), String> {
        for line in lines {
            match line.stream {
                LogStream::Stdout => println!("{}", line),
                LogStream::Stderr => eprintln!("{}", line),
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FileWriter
// ---------------------------------------------------------------------------

/// Writes log lines to a rotating file at `<dir>/<timestamp>.log`.
/// Rotation is triggered by a global line count across all services.
pub struct FileWriter {
    dir: PathBuf,
    max_lines: usize,
    current_file: Option<std::fs::File>,
    line_count: usize,
}

impl FileWriter {
    pub fn new(dir: PathBuf, max_lines: usize) -> Self {
        Self {
            dir,
            max_lines,
            current_file: None,
            line_count: 0,
        }
    }

    fn open_new(&mut self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("failed to create log dir: {}", e))?;
        let ts = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
        let path = self.dir.join(format!("{}.log", ts));
        let file = std::fs::File::create(&path)
            .map_err(|e| format!("failed to create log file: {}", e))?;
        self.current_file = Some(file);
        Ok(())
    }
}

impl LogWriter for FileWriter {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), String> {
        for line in lines {
            if self.current_file.is_none() {
                self.open_new()?;
            }
            if self.line_count >= self.max_lines {
                self.current_file = None;
                self.line_count = 0;
                self.open_new()?;
            }

            let file = self.current_file.as_mut().unwrap();
            // Format as a LogLine<File> without the colour codes.
            let file_line = LogLine::<File> {
                service: line.service.clone(),
                stream: line.stream.clone(),
                text: line.text.clone(),
                timestamp: line.timestamp,
                _marker: PhantomData,
            };
            writeln!(file, "{}", file_line).map_err(|e| format!("log write error: {}", e))?;
            self.line_count += 1;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        if let Some(ref mut file) = self.current_file {
            file.flush()
                .map_err(|e| format!("log flush error: {}", e))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BothWriter
// ---------------------------------------------------------------------------

/// Writes to both terminal and file.
pub struct BothWriter {
    terminal: TerminalWriter,
    file: FileWriter,
}

impl BothWriter {
    pub fn new(terminal: TerminalWriter, file: FileWriter) -> Self {
        Self { terminal, file }
    }
}

impl LogWriter for BothWriter {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), String> {
        self.terminal.write(lines)?;
        self.file.write(lines)
    }

    fn flush(&mut self) -> Result<(), String> {
        self.terminal.flush()?;
        self.file.flush()
    }
}

// ---------------------------------------------------------------------------
// Writer loop  (runs in a dedicated tokio task)
// ---------------------------------------------------------------------------

/// Receives `LogLine`s from the channel and passes them in batches to
/// the configured `LogWriter`.
pub async fn writer_loop(mut rx: mpsc::Receiver<LogLine>, mut writer: Box<dyn LogWriter>) {
    while let Some(line) = rx.recv().await {
        let mut batch = vec![line];
        while let Ok(line) = rx.try_recv() {
            batch.push(line);
        }

        if let Err(e) = writer.write(&batch) {
            eprintln!("logger error: {}", e);
        }
    }

    if let Err(e) = writer.flush() {
        eprintln!("logger flush error: {}", e);
    }
}
