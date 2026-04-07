//! # VMM — hypervisor abstraction layer
//!
//! Wraps per-VM hypervisor interaction behind three traits (ADR-001, ADR-004, ADR-009).
//! CH manages one VM per process with a REST/unix-socket API — this module hides
//! that so the rest of the worker never touches HTTP, sockets, or OS processes directly.
//!
//! ## Three traits ([`interface`])
//!
//! - **[`Vmm`]** — per-VM REST client. One instance = one socket = one VM.
//!   Methods: `create`, `boot`, `shutdown`, `delete`, `info`, `pause`, `resume`, `ping`.
//! - **[`VmmProcess`]** — OS process handle. `kill()` + `cleanup()` (socket, disk copy, logs).
//! - **[`VmmBackend`]** — factory. `prepare()` → `spawn()` → `build_config()`.
//!   Generic over `Client: Vmm` + `Process: VmmProcess`.
//!
//! Three traits (not one) for separation of concerns and testability: tests swap
//! in [`mock::MockBackend`] — no real CH binary, no sockets, no disk I/O.
//!
//! ## vs [`vm_manager`](crate::vm_manager)
//!
//! `vmm` = **driver** for one hypervisor process.
//! `vm_manager` = **fleet manager** that owns N drivers and routes commands to them.
//!
//! ## Modules
//!
//! - [`cloud_hypervisor`] — production CH implementation
//! - [`mock`] — test-only stub (`#[cfg(test)]`)

pub mod cloud_hypervisor;
mod interface;
#[cfg(test)]
pub mod mock;

pub use cloud_hypervisor::CloudHypervisorBackend;
pub use interface::{Vmm, VmmBackend, VmmProcess};
