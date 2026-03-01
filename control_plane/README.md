# Control Plane — Cluster Orchestrator

## What

The master node of the procurator cluster. Runs a Cap'n Proto RPC server implementing the `Master` interface. Accepts connections from the CLI and workers, stores desired state, and schedules VMs to worker nodes.

## Why

Something needs to decide *which* worker runs *which* VM and track whether the cluster has converged to the desired state. The control plane is that coordinator — it receives deployment artifacts from CI, computes assignments, and pushes desired state to workers. It's the "API server + scheduler" equivalent in a Kubernetes analogy.

## Architecture

```
CLI / CI Service
       │
  Cap'n Proto RPC
       │
    Server (stateless RPC adapter)
       │  mpsc
    CommandSender → Node (event processing, worker communication)
                      └── Scheduler (VM-to-worker assignment)
```

Desired state is kept in memory — it's always reconstructable from the latest Git commit, so persistence is unnecessary.

## Status

Scaffolded — the RPC server parses all 5 Master methods and the message-passing architecture is in place. The scheduler and handler implementations are stubs.
