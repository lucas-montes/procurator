/// Integration tests for VM manager
///
/// Note: These tests require:
/// - Root privileges or CAP_NET_ADMIN
/// - KVM support
/// - Network bridge setup (run scripts/setup-network.sh first)
///
/// Run with: cargo test --test integration_test -- --test-threads=1

#[cfg(test)]
mod tests {
    use worker::vms::{VmManager, VmManagerConfig, VmId};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    #[ignore] // Requires privileged setup
    fn test_vm_manager_creation() {
        let config = VmManagerConfig {
            metrics_poll_interval: Duration::from_secs(5),
            auto_restart: true,
            vm_artifacts_dir: PathBuf::from("/tmp/procurator-test/vms"),
            network_bridge: "br-test".to_string(),
            vm_subnet_base: "10.200.0.0/16".to_string(),
        };

        let result = VmManager::new(config);
        assert!(result.is_ok(), "Failed to create VM manager: {:?}", result.err());
    }

    #[test]
    #[ignore] // Requires privileged setup and VM image
    fn test_vm_lifecycle() {
        // This test would require a real VM image from Nix store
        // For now, it's a placeholder showing the intended usage

        let config = VmManagerConfig::default();
        let vm_manager = VmManager::new(config).expect("Failed to create VM manager");

        let vm_id = VmId::new("test-vm");
        let hash = "test-hash".to_string();
        let nix_path = PathBuf::from("/nix/store/test-vm-image");

        // This would fail without a real Nix-built VM image
        // let result = vm_manager.create_vm(vm_id.clone(), hash, &nix_path);

        // In a real test:
        // assert!(result.is_ok());
        // let status = vm_manager.get_vm_status(&vm_id).unwrap();
        // assert_eq!(status, VmStatus::Running);
        // vm_manager.remove_vm(&vm_id).unwrap();
    }

    #[test]
    fn test_vm_id_creation() {
        let id = VmId::new("test-vm-123");
        assert_eq!(id.as_str(), "test-vm-123");

        let id2 = VmId::from("another-vm".to_string());
        assert_eq!(id2.as_str(), "another-vm");
    }
}
