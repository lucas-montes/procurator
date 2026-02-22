# Procurator - Distributed VM Orchestration System

## Architecture Overview

Procurator is a GitOps-driven VM orchestration platform that uses NixOS and cloud-hypervisor to manage distributed workloads. The system follows a control plane + worker pattern with declarative cluster configuration.

### Core Components

**Control Plane** (`control_plane/`): Master node managing cluster state via Cap'n Proto RPC
- Receives desired state from CI/CD (`publishState`)
- Assigns VMs to workers (`getAssignment`)
- Collects observability data (`pushData`)
- Exposes CLI inspection interface (`getClusterStatus`)

**Workers** (`worker/`): Bare-metal nodes running VMs via cloud-hypervisor
- Pull assignments from control plane
- Launch/manage microVMs based on NixOS closures
- Push metrics and observed state back to master
- Located at `worker/src/vmm/cloud_hypervisor.rs`

**Autonix** (`autonix/`): Repository analyzer that detects languages, dependencies, and services
- Scans codebases to extract configuration (`autonix/src/repo/scan.rs`)
- Generates Nix flakes automatically (`autonix/src/repo/flake.rs`)
- Maps languages, lockfiles, containers, CI/CD configs (`autonix/src/mapping/`)

**CI Service** (`ci_service/`): Build and test pipeline orchestrator
- Runs tests, linting, validation on pushed code
- Publishes built closures to binary cache
- Triggers deployments to control plane

**Repohub** (`repohub/`): Web UI for project/repo management
- SQLite-backed repository tracking
- Manages users, projects, repositories
- Displays flake configs and build status

**CLI** (`cli/`): Command-line interface for cluster inspection
- `pcr-test` binary for manual RPC testing (`cli/TESTING.md`)
- Interactive TUI for cluster visualization (`cli/src/interactive/`)

## Communication Protocol

All inter-service RPC uses **Cap'n Proto** schemas in `commands/schema/`:
- `common.capnp`: Shared types (VmSpec, WorkerMetrics, Generation)
- `master.capnp`: Control plane interface (Master capability)
- `worker.capnp`: Worker interface (Worker capability)

Build step: `capnpc` compiles `.capnp` files to Rust in `build.rs` (see `commands/build.rs`, `worker/build.rs`)

## GitOps Workflow

1. **Edit flake**: User modifies `example/flake.nix` defining cluster topology
2. **CI evaluates**: `nix eval --json ".#blueprintJSON" > blueprint.json` serializes desired state
3. **CI builds closures**: `nix build ".#nixosConfigurations.<vm>.config.system.build.toplevel"`
4. **Publish to cache**: `attic push` or `nix copy --to ssh-ng://cache` distributes binaries
5. **Notify control plane**: CI calls `publishState()` RPC with generation + store paths
6. **Control plane schedules**: Assigns VMs to workers based on resources/constraints
7. **Workers pull & activate**: `nix copy --from <cache>` + `switch-to-configuration test`
8. **Health checks**: Workers validate deployment, rollback on failure

See `nix/README.md` for detailed command reference.

## Nix Integration

**Key Modules** (`nix/modules/`):
- `cluster.nix`: Defines `cluster.vms` topology with CPU/memory/deployment config
- `procurator-control-plane.nix`: Systemd service for master node
- `procurator-worker.nix`: Systemd service for worker nodes
- Workers reference control plane by name: `services.procurator.worker.master = "control-plane-1"`

**Store Paths vs Closures**: A closure is a store path + all dependencies. Workers pull complete closures from cache to avoid rebuilding. See `nix/README.md` for cache protocol details.

## Development Workflow

### Building
```bash
cargo build                    # Build all workspace members
cargo build -p control_plane   # Build specific crate
cargo build --bin pcr-test     # Build CLI test binary
```

### Testing
```bash
cargo test                     # Run unit tests (see repo_outils, autonix tests)
cargo run --bin pcr-test -- state publish -c abc123 -g 1 -i def456 -n 3  # Manual RPC test
```

### Running Services
```bash
# Control plane
cd control_plane && cargo run

# Worker
cd worker && cargo run

# Repohub UI
cd repohub && cargo run  # Starts on localhost:3001

# Test CLI
cd cli && cargo run --bin pcr-test -- control status
```

## Conventions

**Async Runtime**: Tokio with `rt-multi-thread`, `macros`, `net`, `fs`, `sync` features. Use `#[tokio::main]` or `#[tokio::test]`.

**Error Handling**: Custom `Error` enums per module (see `repo_outils/src/nix/commands.rs`). Avoid generic error types.

**Workspace Dependencies**: Shared deps defined in root `Cargo.toml` `[workspace.dependencies]`, referenced with `.workspace = true` in members.

**File Naming**: `mod.rs` for public module interface, separate files for implementation (e.g., `mapping/mod.rs` exports, `mapping/languages.rs` implements).

**Testing**: Tests in `#[cfg(test)] mod tests` blocks. Fixtures in `tests/fixtures/` subdirs (see `autonix/tests/fixtures/`).

## Key Files

- `Cargo.toml`: Workspace with 9 members (cli, commands, control_plane, worker, ci_service, cache, repo_outils, repohub, autonix)
- `example/flake.nix`: Reference cluster configuration showing VM topology
- `nix/modules/cluster.nix`: NixOS module for declarative cluster definition
- `commands/schema/*.capnp`: RPC interface definitions
- `autonix/src/repo/analysis.rs`: Core repository analysis logic
- `control_plane/src/scheduler.rs`: VM-to-worker assignment logic (TODO)
- `worker/src/vmm/cloud_hypervisor.rs`: VMM interaction layer

## Common Pitfalls

**Cap'n Proto builds**: Always run `cargo clean` if schema changes don't take effect—build.rs doesn't always detect updates.

**Nix store paths**: Never hardcode `/nix/store/...` paths. Always use `nix build --print-out-paths` or reference derivation outputs.

**Worker state**: Workers are stateless—they reconcile to master's desired state on every poll. No local persistence of VM state.

**Async deadlocks**: Control plane uses `spawn_local` (single-threaded executor). Workers use multi-threaded. Don't mix blocking code in async contexts.

## Future Work (TODO)

- Scheduler implementation (`control_plane/src/scheduler.rs` is stub)
- Master flake auto-generation (see `autonix/MASTER_FLAKE.md`)
- E2E testing framework (cross-service integration tests)
- Build cache registry (implement `.narinfo`/`.nar` protocol in `cache/`)
- AI documentation agent (auto-generate docs from code changes)
