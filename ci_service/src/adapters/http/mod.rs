//! Inbound HTTP adapter: translates requests into use-case calls.

mod checks;
mod credential;
mod error;
mod router;
mod runs;
mod shared;

pub use router::router;
