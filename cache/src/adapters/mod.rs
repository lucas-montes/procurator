//! Adapters: implementations of the ports, and the inbound HTTP interface.

pub mod fs;
#[cfg(feature = "http")]
pub mod http;
#[cfg(test)]
mod index_contract;
pub mod memory;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod system;
