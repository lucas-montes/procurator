# Quick Start Guide

## Fix Dependency Conflict First

The cloud-hypervisor dependency has a version conflict. Fix it with:

```bash
cd /home/lucas/Projects/cloud-hypervisor

# Option 1: Update to latest
git pull origin main
cargo update

# Option 2: Use stable release
git checkout v40.0
```

## Minimal Working Example

### 1. Setup (one-time)

```bash
# Load KVM
sudo modprobe kvm kvm_intel  # or kvm_amd

# Setup network bridge
cd /home/lucas/Projects/procurator/worker
sudo ./scripts/setup-network.sh

# Fix permissions
sudo chmod 666 /dev/kvm
```

### 2. Create a Test VM Config

```bash
mkdir -p /tmp/test-vm
```

Create `/tmp/test-vm/vm-config.json`:

```json
{
  "cpus": {
    "boot_vcpus": 1,
    "max_vcpus": 1
  },
  "memory": {
    "size": 536870912
  },
  "kernel": {
    "path": "/path/to/vmlinux"
  },
  "cmdline": {
    "args": "console=ttyS0 reboot=k panic=1"
  },
  "disks": [],
  "net": [{
    "tap": "tap-test01",
    "ip": "10.100.0.2",
    "mask": "255.255.0.0",
    "mac": "52:54:00:12:34:56"
  }],
  "serial": {
    "mode": "Null"
  },
  "console": {
    "mode": "Off"
  }
}
```

### 3. Use in Code

```rust
use worker::vms::{VmManager, VmManagerConfig, VmId};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize
    let vm_manager = VmManager::new(VmManagerConfig::default())?;

    // Start metrics polling
    let _metrics = vm_manager.start_metrics_polling();

    // Create VM from Nix store path
    let vm_id = VmId::new("my-first-vm");
    let hash = "sha256-abc123".to_string();
    let nix_path = PathBuf::from("/tmp/test-vm");

    // This runs in a blocking thread automatically
    tokio::task::spawn_blocking(move || {
        vm_manager.create_vm(vm_id, hash, &nix_path)
    }).await??;

    println!("VM created and running!");

    Ok(())
}
```

### 4. Run

```bash
cd /home/lucas/Projects/procurator

# After fixing cloud-hypervisor dependency:
cargo run -p worker
```

## Integration with Your Node

In `worker/src/node.rs`:

```rust
use std::sync::Arc;
use crate::vms::{VmManager, VmManagerConfig};

pub struct Node {
    vm_manager: Arc<VmManager>,
    // ... other fields
}

impl Node {
    pub fn new(/* ... */, vm_manager: Arc<VmManager>) -> Self {
        Self { vm_manager, /* ... */ }
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.node_channel.recv().await {
            match msg.event() {
                NodeEvent::Apply => {
                    // Get desired VMs from master
                    let desired = self.fetch_from_master().await;

                    // Reconcile
                    for vm in desired {
                        let vm_manager = self.vm_manager.clone();
                        tokio::task::spawn_blocking(move || {
                            vm_manager.create_vm(vm.id, vm.hash, &vm.path)
                        }).await??;
                    }
                }
            }
        }
    }
}
```

## Troubleshooting

### "Permission denied: /dev/kvm"
```bash
sudo chmod 666 /dev/kvm
```

### "Failed to create bridge"
```bash
sudo ./scripts/setup-network.sh
```

### "Failed to create TAP device"
```bash
# Run as root or add capability
sudo setcap cap_net_admin+ep target/release/worker
```

### Dependency conflict
```bash
cd ../cloud-hypervisor
cargo update
# Or checkout a stable tag
git checkout v40.0
```

## What's Next?

1. Fix cloud-hypervisor dependency
2. Create VM images with Nix
3. Implement reconciliation in Node
4. Add Cap'n Proto RPC for master communication
5. Test end-to-end

See `IMPLEMENTATION_SUMMARY.md` for full details.
