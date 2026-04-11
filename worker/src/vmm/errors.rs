use std::fmt;

/// Errors returned by Node/VmManager through the oneshot reply.
/// Converted to `capnp::Error` at the RPC boundary in Server.
#[derive(Debug)]
pub enum Error {
    /// The requested VM does not exist in the manager's table
    NotFound(String),
    /// The CloudHypervisor REST call failed
    Hypervisor(String),
    /// The CH process failed to spawn or died unexpectedly
    ProcessFailed(String),
    /// The command channel is closed (Node is down)
    ManagerDown,
    /// Catch-all for unexpected failures
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(id) => write!(f, "VM not found: {id}"),
            Error::Hypervisor(msg) => write!(f, "cloud-hypervisor error: {msg}"),
            Error::ProcessFailed(msg) => write!(f, "process error: {msg}"),
            Error::ManagerDown => write!(f, "VM manager is down"),
            Error::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for capnp::Error {
    fn from(e: Error) -> Self {
        capnp::Error::failed(e.to_string())
    }
}
