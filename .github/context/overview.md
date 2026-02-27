# Procurator — Nix-Native VM Orchestrator

## What This Is

A GitOps-driven VM orchestrator. Think Kubernetes, but replacing containers and YAML with Nix closures and cloud-hypervisor VMs. Git commits produce immutable VM images via Nix; the system continuously reconciles running VMs to match.

**Core invariant:** The cluster converges to a set of Nix derivations produced from a Git commit, evaluated outside the cluster, scheduled deterministically, and executed immutably.

## Tech Stack

- **Language:** Rust (edition 2024)
- **Async runtime:** Tokio
- **RPC:** Cap'n Proto (schemas in `commands/schema/`, zero-copy, capability-based)
- **Hypervisor:** Cloud Hypervisor (one process per VM, REST API over unix socket)
- **Package/Image system:** Nix (flakes, closures, binary cache, content-addressed store)
- **VM images:** NixOS minimal (built by `nix/flake-vmm/`, 500-700MB with kernel + SSH)
- **Logging:** `tracing` (structured JSON)
- **Persistence:** SQLite (repohub), in-memory (control plane — desired state is reconstructable from Git)

## Components

| Crate | Role | Status |
|-------|------|--------|
| `worker` | Manages CH processes on a host, serves RPC for VM lifecycle | **Active focus** |
| `control_plane` | Stores desired state, schedules VMs to workers | Scaffolded |
| `cli` | Read-only inspection + manual RPC testing (`pcr-test`) | Scaffolded |
| `ci_service` | Evaluates Nix, builds closures, publishes to cache, notifies control plane | Scaffolded |
| `repohub` | Web UI for repo/project management | Scaffolded |
| `autonix` | Scans repos, detects stack, generates Nix flakes | Working |
| `commands` | Cap'n Proto schemas + generated Rust code | Working |
| `repo_outils` | Git and Nix utility functions | Working |
| `nix/flake-vmm` | NixOS VM image builder + host networking modules | Working |

## Current Focus: Worker + VMM

The immediate goal is a working worker that can spin up and manage cloud-hypervisor VMs, callable from the CLI for testing.

**Architecture:** Server (stateless RPC) → CommandSender → Node → VmManager<B: VmmBackend> (single-owner state, generic over backend) → CH processes (one per VM). See `context/architecture.md` for the full component diagram.

**Key decisions:** No locks (actor model), all dto structs have private fields with constructor + getters, VmManager is generic over `VmmBackend` for testability, one CH process per VM. See `context/decisions.md` for all ADRs.

## Communication Protocol

All inter-service RPC uses **Cap'n Proto** schemas in `commands/schema/`:
- `common.capnp`: Shared data types (VmSpec with 13 fields including kernelPath/initrdPath/diskImagePath/cmdline, WorkerStatus, VmMetrics, etc.)
- `master.capnp`: Control plane interface (publishState, getAssignment, pushData, getClusterStatus, getWorker)
- `worker.capnp`: Worker interface — flat, two methods: `read` (worker status), `listVms` (list VMs)

Build step: `capnpc` compiles `.capnp` → Rust in `commands/build.rs`.

## GitOps Workflow (full loop, future)

```
Git push → CI evaluates Nix → builds closures → publishes to cache
  → notifies control plane → schedules to workers → workers pull & boot VMs
```

No `apply` command. Git is the only write interface. See `context/decisions.md` ADR-008.

## Nix Integration

- **VM images:** Built by `nix/flake-vmm/flake.nix` — `mkVmImage` / `mkVmFromDrv`
- **Host networking:** `nix/flake-vmm/host-module.nix` — bridge, TAP, NAT, domain allowlisting
- **Guest config:** `nix/flake-vmm/vm-module.nix` — systemd, SSH, virtio, workload entrypoint
- **Cluster definition:** `nix/modules/cluster.nix` — declarative VM topology
- **Store paths:** Workers pull closures with `nix copy --from <cache>`. Content-addressed = deterministic drift detection.

## Development

```nushell
cargo build                    # Build all workspace members
cargo build -p worker          # Build worker only
cargo run -p worker            # Run worker (listens on 127.0.0.1:6000)
cargo run --bin pcr-test       # CLI test binary (master)
cargo run --bin pcr-worker-test # CLI test binary (worker)
cargo test                     # Run unit tests
cargo check --workspace        # Type-check all crates
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
- `worker/src/vmm/interface.rs`: Three VMM abstraction traits (Vmm, VmmProcess, VmmBackend)
- `worker/src/vmm/cloud_hypervisor.rs`: CloudHypervisor production backend
- `worker/src/vm_manager.rs`: Single-owner VM state, generic over VmmBackend
- `worker/src/dto.rs`: Private-field message types with constructors and getters

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
