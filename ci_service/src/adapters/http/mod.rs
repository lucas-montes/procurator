//! Inbound HTTP adapter: translates requests into use-case calls.

mod checks;
mod error;
mod router;
mod runs;

pub use router::router;
