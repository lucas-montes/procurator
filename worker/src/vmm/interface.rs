//! VMM abstraction layer for managing virtual machines
//!
//! This module provides a pluggable interface for different VMM backends (cloud-hypervisor, etc.)

use std::fmt::Debug;

pub trait Vmm: Clone + 'static {
    /// VMM-specific configuration type
    type Config: Debug;
    /// VMM-specific info type
    type Info: Debug;
    /// VMM-specific error type
    type Error: std::error::Error;
    /// Create and optionally boot a new VM
    async fn create(&self, config: Self::Config, boot: bool) -> Result<(), Self::Error>;

    /// Delete an existing VM (shutdown if running, then destroy)
    async fn delete(&self, vm_id: &str) -> Result<(), Self::Error>;

    /// Get information about a VM including its metrics
    async fn info(&self, vm_id: &str) -> Result<Self::Info, Self::Error>;

    /// List all VMs managed by this VMM
    async fn list(&self) -> Result<Vec<String>, Self::Error>;
}
