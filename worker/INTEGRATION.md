# Integrating VmManager with Worker Node

This guide shows how to integrate the `VmManager` into your worker's `Node` for VM reconciliation.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Worker Process                       │
│                                                         │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Tokio Async Runtime                      │  │
│  │                                                  │  │
│  │  ┌──────────────┐         ┌─────────────────┐   │  │
│  │  │    Server    │         │      Node       │   │  │
│  │  │ (Cap'n Proto)│────────▶│  (Reconciler)   │   │  │
│  │  └──────────────┘         └────────┬────────┘   │  │
│  │                                    │            │  │
│  └────────────────────────────────────┼────────────┘  │
│                                       │               │
│                                       ▼               │
│  ┌────────────────────────────────────────────────┐   │
│  │              VmManager                         │   │
│  │  (Thread-safe, callable from async)            │   │
│  └────────────────────────────────────────────────┘   │
│                       │                               │
│         ┌─────────────┼─────────────┐                 │
│         ▼             ▼             ▼                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐            │
│  │   VM 1   │  │   VM 2   │  │   VM 3   │            │
│  │  Thread  │  │  Thread  │  │  Thread  │            │
│  │  (Vmm)   │  │  (Vmm)   │  │  (Vmm)   │            │
│  └──────────┘  └──────────┘  └──────────┘            │
│                                                       │
└───────────────────────────────────────────────────────┘
```

## Step 1: Update Node Structure

Add `VmManager` to your `Node`:

```rust
// worker/src/node.rs
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use crate::dto::{NodeEvent, NodeMessage};
use crate::vms::{VmManager, VmManagerConfig};

pub struct Node {
    node_channel: Receiver<NodeMessage>,
    master_addr: SocketAddr,
    vm_manager: Arc<VmManager>,
}

impl Node {
    pub fn new(
        node_channel: Receiver<NodeMessage>,
        master_addr: SocketAddr,
        vm_manager: Arc<VmManager>,
    ) -> Self {
        Node {
            node_channel,
            master_addr,
            vm_manager,
        }
    }

    pub async fn run(mut self) {
        tracing::info!(master_addr=?self.master_addr, "Node started");

        // Start metrics polling
        let _metrics_thread = self.vm_manager.start_metrics_polling();

        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply => {
                    // TODO: Implement reconciliation logic
                    self.reconcile().await;
                }
            }
        }
    }

    async fn reconcile(&self) {
        // This will be implemented in Step 2
        todo!("Implement reconciliation")
    }
}
```

## Step 2: Implement Reconciliation Logic

Add reconciliation between desired state (from master) and current state:

```rust
// worker/src/node.rs

use crate::vms::{VmId, VmStatus};
use std::path::PathBuf;
use std::collections::HashSet;

impl Node {
    /// Reconcile desired VMs with running VMs
    async fn reconcile(&self) {
        tracing::info!("Starting VM reconciliation");

        // 1. Get desired state from master
        let desired_vms = self.fetch_desired_state_from_master().await;

        // 2. Get current running VMs
        let current_vms: HashSet<VmId> = self.vm_manager.list_vms()
            .into_iter()
            .collect();

        // 3. Calculate diff
        let desired_vm_ids: HashSet<VmId> = desired_vms.iter()
            .map(|vm| vm.id.clone())
            .collect();

        // VMs to create (in desired but not running)
        let to_create: Vec<_> = desired_vms.iter()
            .filter(|vm| !current_vms.contains(&vm.id))
            .collect();

        // VMs to remove (running but not in desired)
        let to_remove: Vec<_> = current_vms.iter()
            .filter(|id| !desired_vm_ids.contains(id))
            .collect();

        // VMs to update (hash mismatch)
        let to_update: Vec<_> = desired_vms.iter()
            .filter(|vm| current_vms.contains(&vm.id))
            .filter(|vm| self.needs_update(vm))
            .collect();

        // 4. Execute reconciliation
        for vm in to_remove {
            self.remove_vm(vm).await;
        }

        for vm in to_update {
            // Stop old VM and start new one
            self.remove_vm(&vm.id).await;
            self.create_vm(vm).await;
        }

        for vm in to_create {
            self.create_vm(vm).await;
        }

        tracing::info!(
            created = to_create.len(),
            removed = to_remove.len(),
            updated = to_update.len(),
            "Reconciliation complete"
        );
    }

    async fn fetch_desired_state_from_master(&self) -> Vec<DesiredVm> {
        // TODO: Implement Cap'n Proto call to master
        // For now, return empty
        vec![]
    }

    fn needs_update(&self, desired_vm: &DesiredVm) -> bool {
        // Check if running VM hash matches desired hash
        // TODO: Store running VM hashes in VmHandle
        false
    }

    async fn create_vm(&self, vm: &DesiredVm) {
        tracing::info!(vm_id = %vm.id.as_str(), hash = %vm.hash, "Creating VM");

        // Bridge async -> sync
        let vm_manager = self.vm_manager.clone();
        let vm_id = vm.id.clone();
        let hash = vm.hash.clone();
        let nix_path = vm.nix_store_path.clone();

        // Spawn blocking task for VM creation
        let result = tokio::task::spawn_blocking(move || {
            vm_manager.create_vm(vm_id, hash, &nix_path)
        }).await;

        match result {
            Ok(Ok(())) => tracing::info!(vm_id = %vm.id.as_str(), "VM created successfully"),
            Ok(Err(e)) => tracing::error!(vm_id = %vm.id.as_str(), error = ?e, "Failed to create VM"),
            Err(e) => tracing::error!(vm_id = %vm.id.as_str(), error = ?e, "VM creation task panicked"),
        }
    }

    async fn remove_vm(&self, vm_id: &VmId) {
        tracing::info!(vm_id = %vm_id.as_str(), "Removing VM");

        let vm_manager = self.vm_manager.clone();
        let vm_id = vm_id.clone();

        let result = tokio::task::spawn_blocking(move || {
            vm_manager.remove_vm(&vm_id)
        }).await;

        match result {
            Ok(Ok(())) => tracing::info!(vm_id = %vm_id.as_str(), "VM removed successfully"),
            Ok(Err(e)) => tracing::error!(vm_id = %vm_id.as_str(), error = ?e, "Failed to remove VM"),
            Err(e) => tracing::error!(vm_id = %vm_id.as_str(), error = ?e, "VM removal task panicked"),
        }
    }
}

/// Desired VM state from master
struct DesiredVm {
    id: VmId,
    hash: String,
    nix_store_path: PathBuf,
}
```

## Step 3: Update Main to Initialize VmManager

```rust
// worker/src/lib.rs

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{sync::mpsc::channel, task};

use crate::{node::Node, server::Server};
use crate::vms::{VmManager, VmManagerConfig};

pub mod vms;
mod dto;
mod node;
mod server;

pub async fn main(_hostname: String, addr: SocketAddr, master_addr: SocketAddr) {
    // Initialize VM manager
    let vm_config = VmManagerConfig::default();
    let vm_manager = VmManager::new(vm_config)
        .expect("Failed to create VM manager");
    let vm_manager = Arc::new(vm_manager);

    tracing::info!("VM manager initialized");

    let (tx, rx) = channel(100);

    let node = Node::new(rx, master_addr, vm_manager);
    let server = Server::new(tx);

    tracing::info!(?addr, "Starting worker server");

    let node_task = task::spawn(node.run());

    task::LocalSet::new()
        .run_until(async move {
            tracing::info!("Internal localset server");
            let result = task::spawn_local(server.serve(addr)).await;
            match result {
                Ok(Ok(())) => tracing::info!("Worker server stopped gracefully"),
                Ok(Err(err)) => tracing::error!(?err, "Error starting worker server"),
                Err(err) => tracing::error!(?err, "Worker server task panicked"),
            }
        })
        .await;

    if let Err(err) = node_task.await {
        tracing::error!(?err, "Node task panicked");
    }
}
```

## Step 4: Extend NodeEvent for Reconciliation

Update your DTO to support reconciliation triggers:

```rust
// worker/src/dto.rs

use std::path::PathBuf;
use crate::vms::VmId;

pub enum NodeEvent {
    Apply,
    GetAssignment,
    ReportMetrics,
}

// Message from master with VM assignments
pub struct VmAssignment {
    pub vm_id: VmId,
    pub hash: String,
    pub nix_store_path: PathBuf,
    pub generation: u64,
}
```

## Step 5: Add Metrics Reporting

Periodically report VM metrics to master:

```rust
impl Node {
    async fn report_metrics_to_master(&self) {
        let vms = self.vm_manager.list_vms();

        let mut metrics_report = Vec::new();

        for vm_id in vms {
            if let Ok(metrics) = self.vm_manager.get_vm_metrics(&vm_id) {
                metrics_report.push((vm_id, metrics));
            }
        }

        // TODO: Send metrics to master via Cap'n Proto
        tracing::debug!(vm_count = metrics_report.len(), "Reporting metrics to master");
    }

    pub async fn run(mut self) {
        tracing::info!(master_addr=?self.master_addr, "Node started");

        let _metrics_thread = self.vm_manager.start_metrics_polling();

        // Spawn metrics reporting task
        let vm_manager = self.vm_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                // Report metrics
            }
        });

        while let Some(message) = self.node_channel.recv().await {
            match message.event() {
                NodeEvent::Apply => self.reconcile().await,
                // Handle other events...
            }
        }
    }
}
```

## Setup Instructions

### 1. Install System Dependencies

```bash
# On Ubuntu/Debian
sudo apt-get install -y qemu-kvm libvirt-daemon-system bridge-utils

# Load KVM modules
sudo modprobe kvm
sudo modprobe kvm_intel  # or kvm_amd for AMD
```

### 2. Setup Network Bridge

```bash
cd worker
sudo ./scripts/setup-network.sh
```

This creates the bridge and configures networking.

### 3. Build and Run

```bash
# Build the worker
cargo build --release -p worker

# Run with proper privileges
sudo target/release/worker
```

### 4. Testing Locally

For local development/testing:

```bash
# Build a test VM image with Nix
# (You'll need to create a Nix flake for this)
nix build .#testVmImage

# The result should contain:
# - vm-config.json
# - vmlinux (kernel)
# - disk.img (rootfs)

# Test VM creation
cargo test -p worker --test integration_test -- --ignored
```

## Nix Integration

Your Nix flake should produce VM images like this:

```nix
{
  outputs = { self, nixpkgs }: {
    packages.x86_64-linux.testVmImage = pkgs.stdenv.mkDerivation {
      name = "test-vm-image";

      buildInputs = [ pkgs.cloud-hypervisor ];

      buildPhase = ''
        mkdir -p $out

        # Copy kernel
        cp ${pkgs.linux}/bzImage $out/vmlinux

        # Create minimal rootfs
        # ... (build disk.img)

        # Generate vm-config.json
        cat > $out/vm-config.json <<EOF
        {
          "cpus": { "boot_vcpus": 1, "max_vcpus": 1 },
          "memory": { "size": 1073741824 },
          "kernel": { "path": "$out/vmlinux" },
          "disks": [{ "path": "$out/disk.img" }],
          ...
        }
        EOF
      '';
    };
  };
}
```

## Monitoring

Check VM status:

```bash
# View running VMs
ip link show | grep tap-

# Check bridge
ip addr show br-procurator

# View logs
journalctl -u procurator-worker -f

# Check KVM
lsmod | grep kvm
```

## Troubleshooting

### "Permission denied" on /dev/kvm
```bash
sudo chmod 666 /dev/kvm
# Or add user to kvm group
sudo usermod -aG kvm $USER
```

### "Failed to create TAP device"
```bash
# Check permissions
sudo setcap cap_net_admin+ep target/release/worker

# Or run as root
sudo target/release/worker
```

### Bridge doesn't exist
```bash
# Run setup script
sudo ./scripts/setup-network.sh

# Verify
ip link show br-procurator
```

## Next Steps

1. Implement Cap'n Proto schema for VM assignments
2. Add proper error recovery and retry logic
3. Implement VM health checks
4. Add support for VM snapshots
5. Implement live migration between workers
6. Add resource limits and quotas
7. Implement proper seccomp filters for security
8. Add support for GPU passthrough (if needed)

## Security Considerations

- VMs run with KVM isolation
- Network is isolated via bridge
- TODO: Add seccomp filters to restrict syscalls
- TODO: Add AppArmor/SELinux profiles
- TODO: Implement proper authentication for VM console access
