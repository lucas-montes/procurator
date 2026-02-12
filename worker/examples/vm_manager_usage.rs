/// Example demonstrating how to use the VmManager
///
/// This shows how to:
/// - Initialize the VM manager
/// - Create VMs from Nix store paths
/// - Monitor VM metrics
/// - Handle VM lifecycle

use std::path::PathBuf;
use std::time::Duration;
use worker::vms::{VmManager, VmManagerConfig, VmId};

fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create VM manager configuration
    let config = VmManagerConfig {
        metrics_poll_interval: Duration::from_secs(5),
        auto_restart: true,
        vm_artifacts_dir: PathBuf::from("/var/lib/procurator/vms"),
        network_bridge: "br-procurator".to_string(),
        vm_subnet_base: "10.100.0.0/16".to_string(),
    };

    // Create the VM manager
    let vm_manager = VmManager::new(config)?;

    // Start metrics polling in background
    let _metrics_thread = vm_manager.start_metrics_polling();

    // Example 1: Create a VM from a Nix store path
    let vm_id = VmId::new("llm-agent-001");
    let hash = "sha256-abc123def456".to_string();
    let nix_store_path = PathBuf::from("/nix/store/abc123-vm-image");

    vm_manager.create_vm(vm_id.clone(), hash, &nix_store_path)?;

    // Example 2: List all VMs
    let vms = vm_manager.list_vms();
    println!("Running VMs: {:?}", vms);

    // Example 3: Get VM status
    let status = vm_manager.get_vm_status(&vm_id)?;
    println!("VM status: {:?}", status);

    // Example 4: Get VM metrics
    let metrics = vm_manager.get_vm_metrics(&vm_id)?;
    println!("VM metrics: {:?}", metrics);

    // Example 5: Remove a VM
    vm_manager.remove_vm(&vm_id)?;

    Ok(())
}
