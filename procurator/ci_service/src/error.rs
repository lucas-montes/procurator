//! Error Types
//!
//! Defines domain-specific error types for the CI worker and build execution.
//! Errors are structured to provide context about what went wrong (database, process, nix, git).

use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum WorkerError {
    Database(String),
    Process(String),
    Nix(String),
    Git(String),
    Io(std::io::Error),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::Database(msg) => write!(f, "Database error: {}", msg),
            WorkerError::Process(msg) => write!(f, "Process error: {}", msg),
            WorkerError::Nix(msg) => write!(f, "Nix build error: {}", msg),
            WorkerError::Git(msg) => write!(f, "Git error: {}", msg),
            WorkerError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WorkerError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for WorkerError {
    fn from(err: std::io::Error) -> Self {
        WorkerError::Io(err)
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for WorkerError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        WorkerError::Database(err.to_string())
    }
}
