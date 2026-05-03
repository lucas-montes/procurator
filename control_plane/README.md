# Control Plane — Cluster Orchestrator

## What

The master node of the procurator cluster. Runs a Cap'n Proto RPC server implementing the `Master` interface. Accepts connections from the CLI and workers, stores desired state, and schedules VMs to worker nodes.

## Why

Something needs to decide *which* worker runs *which* VM and track whether the cluster has converged to the desired state. The control plane is that coordinator — it receives deployment artifacts from CI, computes assignments, and pushes desired state to workers. It's the "API server + scheduler" equivalent in a Kubernetes analogy.
