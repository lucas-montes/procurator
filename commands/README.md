# Commands — RPC Schema Definitions

## What

Shared Cap'n Proto schema definitions and generated Rust code for all inter-service communication. Three schema files define the protocol:

- **`common.capnp`** — Shared data types: `VmSpec` (8 fields), `WorkerStatus`, `VmMetrics`, `ClusterStatus`, `Assignment`
- **`worker.capnp`** — Worker interface: `read`, `listVms`, `createVm`, `deleteVm`
- **`master.capnp`** — Control plane interface: `publishState`, `getAssignment`, `pushData`, `getClusterStatus`, `getWorker`

## Why

Every service in procurator communicates over Cap'n Proto RPC. This crate is the single source of truth for the wire format — all other crates depend on it. Cap'n Proto provides zero-copy deserialization for performance and a capability-based security model.

## How It Works

`build.rs` invokes `capnpc` at compile time to generate Rust types from `.capnp` files. Downstream crates import the generated structs and interfaces via `commands::*`.

> **Tip:** If schema changes don't take effect, run `cargo clean` — `build.rs` doesn't always detect `.capnp` file updates.
