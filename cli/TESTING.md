# Testing CLI for Procurator Master Server

This document describes how to use the `pcr-test` CLI tool to manually test the Master server RPC interfaces.

## Building

```bash
cd procurator/cli
cargo build --bin pcr-test
```

Or run directly:
```bash
cargo run --bin pcr-test -- [OPTIONS] <COMMAND>
```

## Usage

The test CLI connects to a running Master server and sends RPC requests to test each interface.

### Global Options

- `--addr <ADDR>` - Master server address (default: `127.0.0.1:5000`)

### Commands

#### Master.State Interface (CD Platform)

Test publishing a new generation to the master:

```bash
# Publish generation with sample VMs
pcr-test state publish --commit abc123 --generation 1 --intent-hash def456 --num-vms 3

# Short form
pcr-test state publish -c abc123 -g 1 -i def456 -n 3
```

#### Master.Worker Interface (Worker Synchronization)

Test worker getting assignments:

```bash
# Worker pulls assignment
pcr-test worker get-assignment \
  --worker-id worker-01 \
  --last-seen-generation 0

# Short form
pcr-test worker get-assignment -w worker-01 -l 0
```

Test worker pushing observability data:

```bash
# Worker pushes status and metrics
pcr-test worker push-data \
  --worker-id worker-01 \
  --observed-generation 1 \
  --num-vms 2

# Short form
pcr-test worker push-data -w worker-01 -o 1 -n 2
```

#### Master.Control Interface (CLI Inspection)

Test getting cluster status:

```bash
pcr-test control status
```

Test getting worker capability:

```bash
pcr-test control get-worker --worker-id worker-01
# Short form
pcr-test control get-worker -w worker-01
```

Test getting VM capability:

```bash
pcr-test control get-vm --vm-id vm-123
# Short form
pcr-test control get-vm -v vm-123
```

## Complete Testing Workflow

Here's a complete workflow to test all interfaces:

```bash
# 1. Start the Master server (in another terminal)
cd procurator/control_plane
cargo run

# 2. Test State interface - publish a new generation
pcr-test --addr 127.0.0.1:5000 state publish -c abc123 -g 1 -i def456 -n 2

# 3. Test Worker interface - get assignment
pcr-test worker get-assignment -w worker-01 -l 0

# 4. Test Worker interface - push data
pcr-test worker push-data -w worker-01 -o 1 -n 2

# 5. Test Control interface - get cluster status
pcr-test control status

# 6. Test Control interface - get worker (will fail until implemented)
pcr-test control get-worker -w worker-01

# 7. Test Control interface - get VM (will fail until implemented)
pcr-test control get-vm -v vm-123
```

## Expected Behavior

### Currently Implemented (will return success):
- ✓ `state publish` - Accepts request and logs
- ✓ `worker get-assignment` - Returns "not implemented" error
- ✓ `worker push-data` - Accepts request and logs
- ✓ `control status` - Returns empty cluster status

### Not Yet Implemented (will return errors):
- ✗ `control get-worker` - Returns "Worker lookup not yet implemented"
- ✗ `control get-vm` - Returns "VM lookup not yet implemented"

## Troubleshooting

### Connection refused
```
Error: Connection refused (os error 111)
```
**Solution**: Make sure the Master server is running on the specified address.

### Address already in use
If starting the server fails:
```
Error: Address already in use (os error 98)
```
**Solution**: Stop any existing server or use a different port.

### Enable debug logging
Set `RUST_LOG` environment variable:
```bash
RUST_LOG=debug pcr-test state publish -c abc123 -g 1 -i def456
```

## Architecture Notes

The test CLI demonstrates the Cap'n Proto RPC architecture:

1. **Master Interface**: The server implements the main `Master` interface with three methods:
   - `getState()` - Returns the State capability
   - `getWorker()` - Returns the Worker capability
   - `getControl()` - Returns the Control capability

2. **Nested Interfaces**: Each nested interface (State, Worker, Control) is implemented by the same `Server` struct but exposed as different capabilities.

3. **Client Pattern**: Clients connect to the Master bootstrap and then request the specific interface they need.

## Example Output

```bash
$ pcr-test state publish -c abc123 -g 1 -i def456 -n 2
2026-02-08T10:30:45.123Z INFO  testing: Connecting to Master server addr=127.0.0.1:5000
2026-02-08T10:30:45.234Z INFO  testing: Connected successfully
2026-02-08T10:30:45.235Z INFO  testing: Testing Master.State.publish() commit=abc123 generation=1 intent_hash=def456 num_vms=2
2026-02-08T10:30:45.236Z DEBUG testing: Providing State interface
2026-02-08T10:30:45.237Z INFO  testing: Sending publish request...
2026-02-08T10:30:45.238Z INFO  server: Publish request generation=1 commit="abc123" intent_hash="def456"
2026-02-08T10:30:45.239Z INFO  testing: ✓ Publish succeeded
```
