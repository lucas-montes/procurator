//! Inbound HTTP adapter: translates requests into use-case calls.

mod admin;
mod auth;
mod error;
mod nix;
mod ops;
mod request_log;
mod router;

pub use router::router;
