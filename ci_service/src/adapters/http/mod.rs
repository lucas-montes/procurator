//! Inbound HTTP adapter: translates requests into use-case calls.

mod error;
mod router;
mod runs;

pub use router::router;
