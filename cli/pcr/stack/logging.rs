use std::fmt;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

const COLORS: &[&str] = &[
    /* ... unchanged ... */
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

pub struct Terminal;
pub struct File;

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
pub struct LogLine<T = Terminal> {
    pub service: String,
    pub stream: LogStream,
    pub text: String,
    pub timestamp: DateTime<Utc>,
    pub _marker: PhantomData<T>,
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

impl fmt::Display for LogLine<Terminal> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = ColoredPrefix::new(&self.service);
        match self.stream {
            LogStream::Stdout => write!(f, "{} {}", prefix, self.text),
            LogStream::Stderr => write!(f, "{} ERR {}", prefix, self.text),
        }
    }
}

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
// LoggingError
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// LogWriter trait
// ---------------------------------------------------------------------------

pub trait LogWriter: Send {
    fn write(&mut self, lines: &[LogLine]) -> Result<(), LoggingError>;
    fn flush(&mut self) -> Result<(), LoggingError>;
}

// ---------------------------------------------------------------------------
// TerminalWriter
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// FileWriter
// ---------------------------------------------------------------------------

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

            let file_line = LogLine::<File> {
                service: line.service.clone(),
                stream: line.stream.clone(),
                text: line.text.clone(),
                timestamp: line.timestamp,
                _marker: PhantomData,
            };
            writeln!(self.current_file, "{}", file_line)?;
            self.line_count += 1;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), LoggingError> {
        self.current_file.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BothWriter
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Writer loop
// ---------------------------------------------------------------------------

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
