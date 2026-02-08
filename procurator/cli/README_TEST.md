# Manual Test CLI

Simple CLI for manually testing and debugging Procurator RPC operations.

## Usage

```bash
# Get cluster status
cargo run --bin pcr-test -- --addr 127.0.0.1:5000 status

# Get worker info
cargo run --bin pcr-test -- worker worker-1

# Get VM info
cargo run --bin pcr-test -- vm vm-1

# Get VM logs
cargo run --bin pcr-test -- logs vm-1
cargo run --bin pcr-test -- logs vm-1 --follow --tail 50

# Get worker assignment
cargo run --bin pcr-test -- assignment worker-1
cargo run --bin pcr-test -- assignment worker-1 --generation 5

# Push observed state
cargo run --bin pcr-test -- push worker-1
cargo run --bin pcr-test -- push worker-1 --generation 2
```

## Commands

- **status** - Get cluster status
- **worker <id>** - Get worker information
- **vm <id>** - Get VM information
- **logs <id>** - Get VM logs (--follow, --tail N)
- **assignment <id>** - Get worker assignment (--generation N)
- **push <id>** - Push observed state (--generation N)

## Options

- `--addr, -a` - Server address (default: 127.0.0.1:5000)

## Enable Debug Logging

```bash
RUST_LOG=debug cargo run --bin pcr-test -- status
```
