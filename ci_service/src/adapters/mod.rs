//! Adapters: implementations of the ports, and the inbound HTTP interface.

#[cfg(feature = "http")]
pub mod http;
pub mod memory;
pub mod nix;
#[cfg(test)]
pub mod scripted;
#[cfg(feature = "sqlite")]
pub mod sqlite;
#[cfg(test)]
mod store_contract;
pub mod system;
