mod errors;
mod interfaces;
mod registry;
mod supervisor;

pub use interfaces::{Factory, Handle, HandleError};
/// Writer-side registry for tests (production code never names it, so the
/// re-export stays test-gated to keep strict lints clean).
#[cfg(test)]
pub use registry::Writer;
pub use registry::{Reader, Registry};
pub use supervisor::{Command, CreateCommand, Supervisor};

pub use errors::Error;
