# Commands — RPC Schema Definitions

## What

Shared Cap'n Proto schema definitions and generated Rust code for all inter-service communication. Four schema files define the protocol:

- **`common.capnp`** — Shared data types: `VmSpec`, `WorkerStatus`, `VmMetrics`, `ClusterStatus`, `Assignment`
- **`worker.capnp`** — Worker interface: `read`, `listVms`, `createVm`, `deleteVm`, `pauseVm`, `resumeVm`, `snapshotVm`, `backupDisk`
- **`master.capnp`** — Control plane interface: `publishState`, `getAssignment`, `pushData`, `getClusterStatus`, `getWorker`
- **`ch.capnp`** — Cloud Hypervisor `VmConfig` (`CpusConfig`, `MemoryConfig`, `PayloadConfig`, `DiskConfig`, `NetConfig`, `ConsoleConfig`); the concrete `BackendConfig` that parameterizes `VmSpec`/`Assignment`/`Worker` in the schemas above

## Why

Every service in procurator communicates over Cap'n Proto RPC. This crate is the single source of truth for the wire format — all other crates depend on it. Cap'n Proto provides zero-copy deserialization for performance and a capability-based security model.

## How It Works

`build.rs` invokes `capnpc` at compile time to generate Rust types from `.capnp` files. Downstream crates import the generated structs and interfaces via `commands::*`.

> **Tip:** If schema changes don't take effect, run `cargo clean` — `build.rs` doesn't always detect `.capnp` file updates.

## Status

Complete for the current protocol surface: all four schemas compile and are consumed by `control_plane` and `worker`, including the VM lifecycle RPCs (`pauseVm`/`resumeVm`/`snapshotVm`/`backupDisk`) implemented in `worker/src/server.rs`. No missing interface or schema is known at this time; the only gap found during this audit was documentation drift (this README previously omitted `ch.capnp` and 4 of the 8 `Worker` RPCs), now corrected above.
