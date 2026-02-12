# Procurator Worker

The worker component of Procurator that manages VMs using cloud-hypervisor.

## Architecture

The worker integrates cloud-hypervisor as a library to manage multiple VMs:

- **One hypervisor instance** shared across all VMs
- **One Vmm thread per VM** running cloud-hypervisor's control loop
- **Async/sync bridge** between tokio runtime (for master communication) and sync OS threads (for VMMs)
- **Automatic networking** using TAP devices and bridges
- **Metrics polling** for resource monitoring

## VM Lifecycle

```
Create → Boot → Running → (Pause/Resume) → Shutdown → Removed
                    ↓
                 Failed → Auto-restart (if enabled)
```

## Usage

### Basic Setup

```rust
use worker::vms::{VmManager, VmManagerConfig, VmId};
use std::path::PathBuf;

// Create VM manager
let config = VmManagerConfig::default();
let vm_manager = VmManager::new(config)?;

// Start metrics polling
let _metrics_thread = vm_manager.start_metrics_polling();

// Create a VM from Nix store path
let vm_id = VmId::new("my-vm");
let hash = "sha256-...";
let nix_path = PathBuf::from("/nix/store/xxx-vm-image");

vm_manager.create_vm(vm_id, hash, &nix_path)?;
```

### VM Configuration

VMs are configured via `vm-config.json` files in the Nix store path. See `examples/vm-config.json` for a complete example.

The Nix derivation should produce:
```
/nix/store/xxx-vm-image/
├── vm-config.json       # VM configuration
├── vmlinux              # Linux kernel (or path reference)
└── disk.img             # Root filesystem image
```

### Networking

The worker automatically:
1. Creates a network bridge (`br-procurator` by default)
2. Allocates IPs from subnet (10.100.0.0/16 by default)
3. Creates TAP devices for each VM
4. Attaches TAP devices to the bridge

VMs are accessible via their assigned IPs. For SSH access, ensure:
- The VM image has SSH server installed and enabled
- The VM's network interface is configured (usually via kernel cmdline or cloud-init)

### Metrics

The worker polls VM metrics at regular intervals:
- CPU usage percentage
- Memory usage
- Network I/O (RX/TX bytes)
- Disk I/O (read/write bytes)

Access metrics via:
```rust
let metrics = vm_manager.get_vm_metrics(&vm_id)?;
println!("Memory: {} MB", metrics.memory_mb);
```

## Integration with Worker Node

The `VmManager` should be integrated into the worker's `Node` struct:

```rust
pub struct Node {
    node_channel: Receiver<NodeMessage>,
    master_addr: SocketAddr,
    vm_manager: Arc<VmManager>,
}
```

When the master sends VM assignments:
1. Compare desired VMs (from master) with running VMs
2. Create missing VMs via `vm_manager.create_vm()`
3. Remove extra VMs via `vm_manager.remove_vm()`
4. Report VM status and metrics to master

## Requirements

### System Requirements

- Linux with KVM support
- Root privileges (or CAP_NET_ADMIN for networking)
- `ip` command available (from iproute2)

### Kernel Modules

```bash
# Load KVM module
modprobe kvm
modprobe kvm_intel  # or kvm_amd

# Load TUN/TAP module
modprobe tun
```

### Permissions

The worker needs:
- Access to `/dev/kvm`
- Permission to create network devices
- Permission to create TAP devices

## Configuration

### VmManagerConfig

```rust
VmManagerConfig {
    // How often to poll VMs for metrics
    metrics_poll_interval: Duration::from_secs(5),

    // Auto-restart failed VMs
    auto_restart: true,

    // Directory for VM artifacts
    vm_artifacts_dir: PathBuf::from("/var/lib/procurator/vms"),

    // Network bridge name
    network_bridge: "br-procurator".to_string(),

    // VM subnet
    vm_subnet_base: "10.100.0.0/16".to_string(),
}
```

## Error Handling

The worker automatically:
- Restarts failed VMs (if `auto_restart` is enabled)
- Cleans up TAP devices on VM shutdown
- Reports errors to logs

Failed VMs enter `VmStatus::Failed(reason)` state.

## TODO

- [ ] Implement proper CPU/memory metrics collection
- [ ] Add guest agent support for better metrics
- [ ] Implement VM migration support
- [ ] Add resource limits enforcement
- [ ] Support for custom network topologies
- [ ] Implement snapshot/restore functionality
- [ ] Add support for multiple disk devices
- [ ] Implement proper seccomp filters
- [ ] Add health checks for VMs
- [ ] Support for serial console access

## Examples

See `examples/vm_manager_usage.rs` for a complete example.

Run with:
```bash
cargo run --example vm_manager_usage
```

## License

Same as the main Procurator project.
