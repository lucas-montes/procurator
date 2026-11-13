//! Why a use case refused or failed.

use std::fmt;

use super::ports::{BlobError, IndexError};
use crate::domain::access::Denied;
use crate::domain::nix::NixError;

/// Why a use case refused or failed.
#[derive(Debug)]
pub enum ServiceError {
    /// Credentials are required, or the ones presented are not valid.
    Unauthenticated,
    /// The caller is known but lacks the grant.
    Forbidden,
    /// No cache has that name.
    UnknownCache,
    /// The request is malformed; the message is safe to show the client.
    Invalid(&'static str),
    /// A narinfo, store hash, or NAR file name was rejected.
    Nix(NixError),
    /// Uploaded bytes do not hash to the name they were uploaded under.
    HashMismatch,
    /// The narinfo names a NAR this cache does not hold.
    NarNotUploaded,
    /// The upload exceeds the per-object size limit.
    TooLarge,
    /// The cache's quota is exhausted.
    QuotaExceeded,
    /// The client's upload stream broke.
    Interrupted,
    /// A port failed; details are logged, not shown to clients.
    Internal(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthenticated => f.write_str("authentication required"),
            Self::Forbidden => f.write_str("not permitted"),
            Self::UnknownCache => f.write_str("no such cache"),
            Self::Invalid(m) => f.write_str(m),
            Self::Nix(e) => e.fmt(f),
            Self::HashMismatch => f.write_str("content does not match its file name"),
            Self::NarNotUploaded => f.write_str("upload the NAR before its narinfo"),
            Self::TooLarge => f.write_str("object too large"),
            Self::QuotaExceeded => f.write_str("cache quota exceeded"),
            Self::Interrupted => f.write_str("upload interrupted"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<Denied> for ServiceError {
    fn from(denied: Denied) -> Self {
        match denied {
            Denied::Unauthenticated => Self::Unauthenticated,
            Denied::Forbidden => Self::Forbidden,
        }
    }
}

impl From<NixError> for ServiceError {
    fn from(error: NixError) -> Self {
        Self::Nix(error)
    }
}

impl From<IndexError> for ServiceError {
    fn from(error: IndexError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<BlobError> for ServiceError {
    fn from(error: BlobError) -> Self {
        match error {
            BlobError::TooLarge => Self::TooLarge,
            BlobError::Interrupted(_) => Self::Interrupted,
            BlobError::Backend(m) => Self::Internal(m),
        }
    }
}
