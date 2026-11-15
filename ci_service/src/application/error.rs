//! Use-case failures.

use std::fmt;

use super::ports::StoreError;
use crate::domain::run::{InvalidRun, RunStatus};

/// Why a use case refused or failed.
#[derive(Debug)]
pub enum ServiceError {
    /// The input was rejected.
    Invalid(InvalidRun),
    /// No such run.
    NotFound,
    /// The run is not in a status that allows this change.
    Conflict(RunStatus),
    /// A status that does not end a run was given to `finish`.
    NotFinal(RunStatus),
    /// A port failed; details are for logs, not for callers.
    Internal(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => reason.fmt(f),
            Self::NotFound => f.write_str("run not found"),
            Self::Conflict(status) => write!(f, "run is {}", status.as_str()),
            Self::NotFinal(status) => write!(f, "`{}` does not end a run", status.as_str()),
            Self::Internal(detail) => write!(f, "internal error: {detail}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<InvalidRun> for ServiceError {
    fn from(reason: InvalidRun) -> Self {
        Self::Invalid(reason)
    }
}

impl From<StoreError> for ServiceError {
    fn from(error: StoreError) -> Self {
        Self::Internal(error.to_string())
    }
}
