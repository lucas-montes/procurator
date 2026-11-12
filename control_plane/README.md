# Control Plane — Cluster Orchestrator

## What

The master node of the procurator cluster. Runs a Cap'n Proto RPC server implementing the `Master` interface. Accepts connections from the CLI and workers, stores desired state, and schedules VMs to worker nodes.

## Why

Something needs to decide *which* worker runs *which* VM and track whether the cluster has converged to the desired state. The control plane is that coordinator — it receives deployment artifacts from CI, computes assignments, and pushes desired state to workers. It's the "API server + scheduler" equivalent in a Kubernetes analogy.

## Status

Functional for explicit, pull-based assignment: `NodeState` stores generations with commit/intent-hash/spec data, the `Master` RPC (`publishState`, `getAssignment`, `pushData`, `getClusterStatus`, `getWorker`) is wired end to end, and workers poll and heartbeat against it.

Missing to reach the full "scheduler" goal:

- **Persistence** — all state (`registry.rs`, `store.rs`) lives in memory only; a control-plane restart loses every generation and assignment.
- **Worker↔master authentication** — none; relies on a private-network assumption.
- **Per-VM status detail** — `getClusterStatus`/`getWorker` track generation and running-set convergence per worker, but don't surface per-VM status rows.
- **Real scheduling logic** — `scheduler.rs` (`explicit_map`) only groups pre-pinned assignments by their declared `targetWorker`; there is no bin-packing, label selection, or resource-aware placement yet. Every VM must already name its target worker.
