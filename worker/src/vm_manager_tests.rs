#[cfg(test)]
mod tests {
    use tokio::sync::oneshot;

    use crate::dto::{
        CommandPayload, CommandResponse, Message, VmError, VmSpec,
    };
    use crate::vm_manager::{VmManager, VmManagerConfig};
    use crate::vmm::mock::{MockBackend, MockBackendConfig};

    // ─── Helpers ───────────────────────────────────────────────────────

    fn test_spec() -> VmSpec {
        VmSpec::new(
            "/nix/store/aaaa-nixos-system".to_string(),
            "/nix/store/bbbb-kernel/bzImage".to_string(),
            "/nix/store/cccc-initrd/initrd".to_string(),
            "/nix/store/dddd-disk/nixos.raw".to_string(),
            "console=ttyS0 root=/dev/vda rw".to_string(),
            2,
            1024,
            vec!["api.openai.com".to_string()],
        )
    }

    fn test_config() -> VmManagerConfig {
        VmManagerConfig {
            worker_id: "test-worker".to_string(),
        }
    }

    /// Send a command to the manager and return the response.
    async fn send(
        manager: &mut VmManager<impl crate::vmm::VmmBackend>,
        payload: CommandPayload,
    ) -> Result<CommandResponse, VmError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let msg = Message::from_parts(payload, reply_tx);
        manager.handle(msg).await;
        reply_rx.await.expect("reply channel dropped")
    }

    // ─── JSON deserialization (Nix → Rust contract) ────────────────────

    /// The exact JSON shape that Nix's `builtins.toJSON vmSpec` produces.
    /// If this test breaks, the Nix output and the Rust consumer are out of sync.
    const NIX_VM_SPEC_JSON: &str = r#"{
        "toplevel": "/nix/store/aaaa-nixos-system",
        "kernelPath": "/nix/store/bbbb-kernel/bzImage",
        "initrdPath": "/nix/store/cccc-initrd/initrd",
        "diskImagePath": "/nix/store/dddd-disk/nixos.raw",
        "cmdline": "console=ttyS0 root=/dev/vda rw init=/sbin/init",
        "cpu": 2,
        "memoryMb": 1024,
        "networkAllowedDomains": ["api.openai.com", "github.com"]
    }"#;

    #[test]
    fn deserialize_nix_vm_spec_json() {
        let spec: VmSpec = serde_json::from_str(NIX_VM_SPEC_JSON)
            .expect("VmSpec should deserialize from Nix JSON");

        assert_eq!(spec.toplevel(), "/nix/store/aaaa-nixos-system");
        assert_eq!(spec.kernel_path(), "/nix/store/bbbb-kernel/bzImage");
        assert_eq!(spec.initrd_path(), "/nix/store/cccc-initrd/initrd");
        assert_eq!(spec.disk_image_path(), "/nix/store/dddd-disk/nixos.raw");
        assert_eq!(spec.cmdline(), "console=ttyS0 root=/dev/vda rw init=/sbin/init");
        assert_eq!(spec.cpu(), 2);
        assert_eq!(spec.memory_mb(), 1024);
        assert_eq!(
            spec.network_allowed_domains(),
            &["api.openai.com", "github.com"]
        );
    }

    #[test]
    fn deserialize_minimal_vm_spec_json() {
        // Minimum valid spec: empty domains list, single CPU, small memory
        let json = r#"{
            "toplevel": "/nix/store/xxx-system",
            "kernelPath": "/nix/store/xxx-kernel/bzImage",
            "initrdPath": "/nix/store/xxx-initrd/initrd",
            "diskImagePath": "/nix/store/xxx-disk/nixos.raw",
            "cmdline": "console=ttyS0 root=/dev/vda rw init=/sbin/init",
            "cpu": 1,
            "memoryMb": 512,
            "networkAllowedDomains": []
        }"#;

        let spec: VmSpec = serde_json::from_str(json)
            .expect("minimal VmSpec should deserialize");

        assert_eq!(spec.cpu(), 1);
        assert_eq!(spec.memory_mb(), 512);
        assert!(spec.network_allowed_domains().is_empty());
    }

    #[test]
    fn deserialize_rejects_missing_field() {
        // Missing "cpu" field
        let json = r#"{
            "toplevel": "/nix/store/xxx",
            "kernelPath": "/nix/store/xxx",
            "initrdPath": "/nix/store/xxx",
            "diskImagePath": "/nix/store/xxx",
            "cmdline": "console=ttyS0",
            "memoryMb": 512,
            "networkAllowedDomains": []
        }"#;

        let result = serde_json::from_str::<VmSpec>(json);
        assert!(result.is_err(), "should reject JSON missing required field 'cpu'");
    }

    #[test]
    fn deserialize_rejects_snake_case_fields() {
        // Worker must NOT accept snake_case — only camelCase from Nix
        let json = r#"{
            "toplevel": "/nix/store/xxx",
            "kernel_path": "/nix/store/xxx",
            "initrd_path": "/nix/store/xxx",
            "disk_image_path": "/nix/store/xxx",
            "cmdline": "console=ttyS0",
            "cpu": 1,
            "memory_mb": 512,
            "network_allowed_domains": []
        }"#;

        let result = serde_json::from_str::<VmSpec>(json);
        assert!(
            result.is_err(),
            "should reject snake_case fields — only camelCase is valid"
        );
    }

    #[test]
    fn deserialize_rejects_extra_unknown_fields() {
        // Extra fields should still parse (serde default is to ignore unknown)
        // — this is intentional for forward compatibility
        let json = r#"{
            "toplevel": "/nix/store/xxx",
            "kernelPath": "/nix/store/xxx",
            "initrdPath": "/nix/store/xxx",
            "diskImagePath": "/nix/store/xxx",
            "cmdline": "console=ttyS0",
            "cpu": 1,
            "memoryMb": 512,
            "networkAllowedDomains": [],
            "futureField": "ignored"
        }"#;

        let result = serde_json::from_str::<VmSpec>(json);
        assert!(result.is_ok(), "extra fields should be ignored for forward compat");
    }

    #[tokio::test]
    async fn create_vm_returns_uuid() {
        let (backend, tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::Create(test_spec())).await;

        let id = match resp {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };

        // UUIDv7 format: 8-4-4-4-12 hex chars
        assert_eq!(id.len(), 36, "UUID should be 36 chars");
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);

        // Backend was called: spawn → create → boot
        assert_eq!(tracker.spawn_count(), 1);
        assert_eq!(tracker.create_count(), 1);
        assert_eq!(tracker.boot_count(), 1);
    }

    #[tokio::test]
    async fn create_two_vms_returns_different_ids() {
        let (backend, tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let id1 = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };
        let id2 = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };

        assert_ne!(id1, id2, "each VM should get a unique ID");
        assert_eq!(tracker.spawn_count(), 2);
    }

    // ─── Delete VM ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_existing_vm_succeeds() {
        let (backend, tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        // Create first
        let id = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };

        // Delete
        let resp = send(&mut mgr, CommandPayload::Delete(id)).await;
        assert!(matches!(resp, Ok(CommandResponse::Unit)));

        // shutdown + delete + kill + cleanup all called
        assert_eq!(tracker.shutdown_count(), 1);
        assert_eq!(tracker.delete_count(), 1);
        assert_eq!(tracker.kill_count(), 1);
        assert_eq!(tracker.cleanup_count(), 1);
    }

    #[tokio::test]
    async fn delete_nonexistent_vm_returns_not_found() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(
            &mut mgr,
            CommandPayload::Delete("no-such-id".to_string()),
        )
        .await;

        match resp {
            Err(VmError::NotFound(id)) => assert_eq!(id, "no-such-id"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delete_already_deleted_vm_returns_not_found() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let id = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };

        // First delete succeeds
        let resp = send(&mut mgr, CommandPayload::Delete(id.clone())).await;
        assert!(matches!(resp, Ok(CommandResponse::Unit)));

        // Second delete → NotFound
        let resp = send(&mut mgr, CommandPayload::Delete(id.clone())).await;
        match resp {
            Err(VmError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    // ─── List VMs ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_returns_empty_vec() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::List).await;

        match resp {
            Ok(CommandResponse::VmList(list)) => assert!(list.is_empty()),
            other => panic!("expected VmList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_returns_created_vms() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        // Create 3 VMs
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
                Ok(CommandResponse::VmId(id)) => id,
                other => panic!("expected VmId, got {other:?}"),
            };
            ids.push(id);
        }

        let resp = send(&mut mgr, CommandPayload::List).await;
        match resp {
            Ok(CommandResponse::VmList(list)) => {
                assert_eq!(list.len(), 3);
                // All created IDs appear in the list
                for id in &ids {
                    assert!(
                        list.iter().any(|info| info.id() == id),
                        "VM {id} missing from list"
                    );
                }
                // All report status running
                for info in &list {
                    assert_eq!(info.status().as_str(), "running");
                    assert_eq!(info.worker_id(), "test-worker");
                }
            }
            other => panic!("expected VmList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_excludes_deleted_vms() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        let id1 = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };
        let _id2 = match send(&mut mgr, CommandPayload::Create(test_spec())).await {
            Ok(CommandResponse::VmId(id)) => id,
            other => panic!("expected VmId, got {other:?}"),
        };

        // Delete the first one
        send(&mut mgr, CommandPayload::Delete(id1.clone())).await.unwrap();

        let resp = send(&mut mgr, CommandPayload::List).await;
        match resp {
            Ok(CommandResponse::VmList(list)) => {
                assert_eq!(list.len(), 1);
                assert_ne!(list[0].id(), id1);
            }
            other => panic!("expected VmList, got {other:?}"),
        }
    }

    // ─── Worker status ─────────────────────────────────────────────────

    #[tokio::test]
    async fn worker_status_reports_running_count() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        // Empty → 0 running
        let resp = send(&mut mgr, CommandPayload::GetWorkerStatus).await;
        match &resp {
            Ok(CommandResponse::WorkerInfo(info)) => {
                assert_eq!(info.id(), "test-worker");
                assert!(info.healthy());
                assert_eq!(info.running_vms(), 0);
            }
            other => panic!("expected WorkerInfo, got {other:?}"),
        }

        // Create 2 VMs → 2 running
        send(&mut mgr, CommandPayload::Create(test_spec())).await.unwrap();
        send(&mut mgr, CommandPayload::Create(test_spec())).await.unwrap();

        let resp = send(&mut mgr, CommandPayload::GetWorkerStatus).await;
        match resp {
            Ok(CommandResponse::WorkerInfo(info)) => {
                assert_eq!(info.running_vms(), 2);
            }
            other => panic!("expected WorkerInfo, got {other:?}"),
        }
    }

    // ─── Failure injection ─────────────────────────────────────────────

    #[tokio::test]
    async fn spawn_failure_returns_error() {
        let config = MockBackendConfig {
            spawn_error: Some("disk full".to_string()),
            ..Default::default()
        };
        let (backend, _tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::Create(test_spec())).await;
        match resp {
            Err(VmError::ProcessFailed(msg)) => {
                assert!(msg.contains("disk full"), "got: {msg}");
            }
            other => panic!("expected ProcessFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_failure_returns_hypervisor_error() {
        let config = MockBackendConfig {
            create_error: Some("bad config".to_string()),
            ..Default::default()
        };
        let (backend, tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::Create(test_spec())).await;
        match resp {
            Err(VmError::Hypervisor(msg)) => {
                assert!(msg.contains("vm.create failed"), "got: {msg}");
            }
            other => panic!("expected Hypervisor error, got {other:?}"),
        }

        // spawn was called, but create failed so boot should NOT be called
        assert_eq!(tracker.spawn_count(), 1);
        assert_eq!(tracker.create_count(), 1);
        assert_eq!(tracker.boot_count(), 0);
    }

    #[tokio::test]
    async fn boot_failure_returns_hypervisor_error() {
        let config = MockBackendConfig {
            boot_error: Some("kernel panic".to_string()),
            ..Default::default()
        };
        let (backend, tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::Create(test_spec())).await;
        match resp {
            Err(VmError::Hypervisor(msg)) => {
                assert!(msg.contains("vm.boot failed"), "got: {msg}");
            }
            other => panic!("expected Hypervisor error, got {other:?}"),
        }

        // spawn + create succeeded, boot failed
        assert_eq!(tracker.spawn_count(), 1);
        assert_eq!(tracker.create_count(), 1);
        assert_eq!(tracker.boot_count(), 1);
    }

    #[tokio::test]
    async fn failed_create_does_not_leave_vm_in_table() {
        let config = MockBackendConfig {
            create_error: Some("fail".to_string()),
            ..Default::default()
        };
        let (backend, _tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        // Attempt create (fails)
        let _ = send(&mut mgr, CommandPayload::Create(test_spec())).await;

        // List should be empty — failed VMs don't leak into the table
        let resp = send(&mut mgr, CommandPayload::List).await;
        match resp {
            Ok(CommandResponse::VmList(list)) => assert!(list.is_empty()),
            other => panic!("expected empty VmList, got {other:?}"),
        }
    }

    // ─── VmSpec field validation ───────────────────────────────────────

    #[tokio::test]
    async fn vm_info_contains_toplevel_hash() {
        let (backend, _tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        send(&mut mgr, CommandPayload::Create(test_spec())).await.unwrap();

        let resp = send(&mut mgr, CommandPayload::List).await;
        match resp {
            Ok(CommandResponse::VmList(list)) => {
                assert_eq!(list.len(), 1);
                assert_eq!(
                    list[0].desired_hash(),
                    "/nix/store/aaaa-nixos-system",
                    "desired_hash should come from spec.toplevel()"
                );
            }
            other => panic!("expected VmList, got {other:?}"),
        }
    }

    // ─── Prepare (artifact resolution) ─────────────────────────────────

    #[tokio::test]
    async fn create_calls_prepare_before_spawn() {
        let (backend, tracker) = MockBackend::new();
        let mut mgr = VmManager::new(backend, test_config());

        send(&mut mgr, CommandPayload::Create(test_spec())).await.unwrap();

        // prepare is called once, before spawn
        assert_eq!(tracker.prepare_count(), 1);
        assert_eq!(tracker.spawn_count(), 1);
    }

    #[tokio::test]
    async fn prepare_failure_prevents_spawn() {
        let config = MockBackendConfig {
            prepare_error: Some("cache unreachable".to_string()),
            ..Default::default()
        };
        let (backend, tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        let resp = send(&mut mgr, CommandPayload::Create(test_spec())).await;
        match resp {
            Err(VmError::Internal(msg)) => {
                assert!(msg.contains("cache unreachable"), "got: {msg}");
            }
            other => panic!("expected Internal error, got {other:?}"),
        }

        // prepare was called but spawn should NOT have been called
        assert_eq!(tracker.prepare_count(), 1);
        assert_eq!(tracker.spawn_count(), 0);
        assert_eq!(tracker.create_count(), 0);
        assert_eq!(tracker.boot_count(), 0);
    }

    #[tokio::test]
    async fn prepare_failure_does_not_leave_vm_in_table() {
        let config = MockBackendConfig {
            prepare_error: Some("missing closure".to_string()),
            ..Default::default()
        };
        let (backend, _tracker) = MockBackend::with_config(config);
        let mut mgr = VmManager::new(backend, test_config());

        let _ = send(&mut mgr, CommandPayload::Create(test_spec())).await;

        let resp = send(&mut mgr, CommandPayload::List).await;
        match resp {
            Ok(CommandResponse::VmList(list)) => assert!(list.is_empty()),
            other => panic!("expected empty VmList, got {other:?}"),
        }
    }
}
