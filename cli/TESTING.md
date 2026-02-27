# Testing CLIs for Procurator

This document describes how to use `pcr-test` (Master server) and `pcr-worker-test` (Worker server) to manually test the RPC interfaces.

## Building

```nushell
# Build both test binaries
cargo build --bin pcr-test --bin pcr-worker-test

# Or run directly
cargo run --bin pcr-test -- [OPTIONS] <COMMAND>
cargo run --bin pcr-worker-test -- [OPTIONS] <COMMAND>
```

---

## pcr-test — Master Server Testing

Connects to a running Master (control plane) server and tests each RPC method.

### Global Options

- `--addr <ADDR>` — Master server address (default: `127.0.0.1:5000`)

### Commands

#### Master.State Interface (CD Platform)

Publish a new generation with sample VMs:

```nushell
# Publish generation with sample VMs (includes kernelPath, initrdPath, diskImagePath, cmdline)
cargo run --bin pcr-test -- state publish --commit abc123 --generation 1 --intent-hash def456 --num-vms 3

# Short form
cargo run --bin pcr-test -- state publish -c abc123 -g 1 -i def456 -n 3
```

#### Master.Worker Interface (Worker Synchronization)

```nushell
# Worker pulls assignment
cargo run --bin pcr-test -- worker get-assignment --worker-id worker-01 --last-seen-generation 0

# Worker pushes status and metrics
cargo run --bin pcr-test -- worker push-data --worker-id worker-01 --observed-generation 1 --num-vms 2
```

#### Master.Control Interface (CLI Inspection)

```nushell
# Get cluster status
cargo run --bin pcr-test -- control status

# Get a specific worker capability
cargo run --bin pcr-test -- control get-worker --worker-id worker-01
```

### Complete Master Testing Workflow

```nushell
# 1. Start the Master server (in another terminal)
cargo run -p control_plane

# 2. Test State interface — publish a new generation
cargo run --bin pcr-test -- --addr 127.0.0.1:5000 state publish -c abc123 -g 1 -i def456 -n 2

# 3. Test Worker interface — get assignment
cargo run --bin pcr-test -- worker get-assignment -w worker-01 -l 0

# 4. Test Worker interface — push data
cargo run --bin pcr-test -- worker push-data -w worker-01 -o 1 -n 2

# 5. Test Control interface — get cluster status
cargo run --bin pcr-test -- control status

# 6. Test Control interface — get worker
cargo run --bin pcr-test -- control get-worker -w worker-01
```

### Expected Behavior (Master)

#### Currently Implemented (will return success):
- ✓ `state publish` — Accepts request and logs
- ✓ `worker push-data` — Accepts request and logs
- ✓ `control status` — Returns empty cluster status

#### Not Yet Implemented (will return errors):
- ✗ `worker get-assignment` — Returns "not implemented" error
- ✗ `control get-worker` — Returns "Worker lookup not yet implemented"

---

## pcr-worker-test — Worker Server Testing

Connects to a running Worker server and tests the Worker RPC interface.

### Global Options

- `--addr <ADDR>` — Worker server address (default: `127.0.0.1:6000`)

### Commands

#### Worker.read — Fetch worker status

```nushell
cargo run --bin pcr-worker-test -- read
cargo run --bin pcr-worker-test -- --addr 127.0.0.1:6000 read
```

#### Worker.listVms — List all VMs

```nushell
cargo run --bin pcr-worker-test -- list-vms
```

### Complete Worker Testing Workflow

```nushell
# 1. Start the Worker server (in another terminal)
cargo run -p worker

# 2. Test reading worker status
cargo run --bin pcr-worker-test -- --addr 127.0.0.1:6000 read

# 3. Test listing VMs (returns empty list until VMs are created via reconciler)
cargo run --bin pcr-worker-test -- list-vms
```

### Expected Behavior (Worker)

- ✓ `read` — Returns worker status (id, generation, running_vms count)
- ✓ `list-vms` — Returns list of VMs (empty until reconciler or create-vm is wired)

---

## Troubleshooting

### Connection refused
```
Error: Connection refused (os error 111)
```
**Solution**: Make sure the target server is running on the specified address.

### Address already in use
```
Error: Address already in use (os error 98)
```
**Solution**: Stop any existing server or use a different port.

### Enable debug logging

```nushell
$env.RUST_LOG = "debug"; cargo run --bin pcr-test -- state publish -c abc123 -g 1 -i def456
$env.RUST_LOG = "debug"; cargo run --bin pcr-worker-test -- read
```

## Architecture Notes

The test CLIs exercise two independent RPC servers:

1. **Master Server** (`control_plane`): Flat interface — `publishState`, `getAssignment`, `pushData`, `getClusterStatus`, `getWorker`. All methods are top-level on the `Master` capability.

2. **Worker Server** (`worker`): Two RPC methods — `read` (worker status) and `listVms` (list VMs). The Server is a stateless adapter that forwards commands through an mpsc channel to the Node/VmManager.

3. **Worker Internal Architecture**: Server (stateless, cloneable) → CommandSender → Node → VmManager (owns all VM state). VmManager is generic over `VmmBackend` for testability (production uses `CloudHypervisorBackend`).

4. **Client Pattern**: Clients connect to the bootstrap capability (Master or Worker) and call methods directly — no nested sub-interfaces.

## Example Output

```nushell
> cargo run --bin pcr-test -- state publish -c abc123 -g 1 -i def456 -n 2
INFO  testing: Connecting to Master server addr=127.0.0.1:5000
INFO  testing: Connected successfully
INFO  testing: Testing Master.State.publish() commit=abc123 generation=1 intent_hash=def456 num_vms=2
INFO  testing: Sending publish request...
INFO  testing: ✓ Publish succeeded
```

```nushell
> cargo run --bin pcr-worker-test -- read
INFO  worker_testing: Connecting to Worker server addr=127.0.0.1:6000
INFO  worker_testing: Connected successfully
INFO  worker_testing: Testing Worker.read()
INFO  worker_testing: ✓ Got worker status worker_id="unknown" generation=0 running_vms=0
```
