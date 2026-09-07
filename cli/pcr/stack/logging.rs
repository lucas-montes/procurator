use std::fmt;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

const COLORS: &[&str] = &[
    "\x1b[31m",
    "\x1b[32m",
    "\x1b[33m",
    "\x1b[34m",
    "\x1b[35m",
    "\x1b[36m",
    "\x1b[91m",
    "\x1b[92m",
    "\x1b[93m",
    "\x1b[94m",
    "\x1b[95m",
    "\x1b[96m",
    "\x1b[38;5;208m",
    "\x1b[38;5;45m",
    "\x1b[38;5;200m",
    "\x1b[38;5;118m",
    "\x1b[38;5;99m",
    "\x1b[38;5;37m",
    "\x1b[38;5;173m",
    "\x1b[38;5;141m",
    "\x1b[38;5;48m",
    "\x1b[38;5;203m",
    "\x1b[38;5;75m",
    "\x1b[38;5;220m",
    "\x1b[38;5;205m",
    "\x1b[38;5;120m",
    "\x1b[38;5;68m",
    "\x1b[38;5;179m",
    "\x1b[38;5;51m",
    "\x1b[38;5;155m",
];
const RESET: &str = "\x1b[0m";

pub fn color_for(name: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = hasher.finish() as usize % COLORS.len();
    COLORS[idx]
}

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

#[derive(Debug, Clone)]
pub struct LogLine {
    pub service: String,
    pub stream: LogStream,
    pub text: String,
    pub timestamp: DateTime<Utc>,
}

impl LogLine {
    pub fn new(service: String, stream: LogStream, text: String, timestamp: DateTime<Utc>) -> Self {
        Self {
            service,
            stream,
            text,
            timestamp,
        }
    }
}

/// Terminal-formatted display: colored prefix + text.
impl fmt::Display for LogLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = ColoredPrefix::new(&self.service);
        match self.stream {
            LogStream::Stdout => write!(f, "{} {}", prefix, self.text),
            LogStream::Stderr => write!(f, "{} ERR {}", prefix, self.text),
        }
    }
}

/// File-formatted display: timestamped structured log.
pub struct FileLogLine(LogLine);

impl From<LogLine> for FileLogLine {
    fn from(line: LogLine) -> Self {
        FileLogLine(line)
    }
}

impl fmt::Display for FileLogLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] [{}] {} {}",
            self.0.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            self.0.service,
            self.0.stream,
            self.0.text,
        )
    }
}

#[derive(Debug)]
pub enum LoggingError {
    Io(std::io::Error),
}

impl fmt::Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoggingError::Io(e) => write!(f, "log I/O error: {}", e),
        }
    }
}

impl std::error::Error for LoggingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoggingError::Io(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for LoggingError {
    fn from(e: std::io::Error) -> Self {
        LoggingError::Io(e)
    }
}

pub trait LogWriter: Send + 'static {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), LoggingError>;
    fn flush(&mut self) -> Result<(), LoggingError>;

    fn spawn(self, size: usize, rx: mpsc::Receiver<LogLine>) -> tokio::task::JoinHandle<()>
    where
        Self: Sized,
    {
        tokio::spawn(writer_loop(size, rx, self))
    }
}

#[derive(Debug, Default)]
pub struct TerminalWriter;

impl LogWriter for TerminalWriter {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), LoggingError> {
        let mut stdout = io::stdout().lock();
        let mut stderr = io::stderr().lock();
        for line in lines {
            match line.stream {
                LogStream::Stdout => writeln!(stdout, "{}", line).map_err(LoggingError::Io)?,
                LogStream::Stderr => writeln!(stderr, "{}", line).map_err(LoggingError::Io)?,
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), LoggingError> {
        io::stdout().flush()?;
        Ok(())
    }
}

pub struct FileWriter {
    dir: PathBuf,
    max_lines: usize,
    current_file: fs::File,
    line_count: usize,
}

impl FileWriter {
    pub fn new(dir: PathBuf, max_lines: usize, current_file: fs::File) -> Self {
        Self {
            dir,
            max_lines,
            current_file,
            line_count: 0,
        }
    }

    pub fn new_file(dir: &Path) -> Result<fs::File, LoggingError> {
        fs::create_dir_all(dir)?;
        let ts = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
        let path = dir.join(format!("{}.log", ts));
        Ok(fs::File::create(&path)?)
    }
}

impl LogWriter for FileWriter {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), LoggingError> {
        for line in lines {
            if self.line_count >= self.max_lines {
                self.line_count = 0;
                self.current_file = FileWriter::new_file(self.dir.as_path())?;
            }
            let fl: FileLogLine = line.clone().into();
            writeln!(self.current_file, "{}", fl)?;
            self.line_count += 1;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), LoggingError> {
        self.current_file.flush()?;
        Ok(())
    }
}

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
    fn write(&mut self, lines: &[LogLine]) -> Result<(), LoggingError> {
        self.terminal.write(lines)?;
        self.file.write(lines)
    }

    fn flush(&mut self) -> Result<(), LoggingError> {
        self.terminal.flush()?;
        self.file.flush()
    }
}

async fn writer_loop<W: LogWriter>(size: usize, mut rx: mpsc::Receiver<LogLine>, mut writer: W) {
    let mut batch = Vec::with_capacity(size);
    while rx.recv_many(&mut batch, size).await != 0 {
        batch.sort_by_key(|line| line.timestamp);
        if let Err(e) = writer.write(&batch) {
            tracing::error!(?e, "error writing log lines");
        }
        batch.clear();
        if let Err(e) = writer.flush() {
            tracing::error!(?e, "error flushing log lines");
        }
    }
}
