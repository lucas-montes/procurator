# CI Service — Build Pipeline

## What

A CI/CD pipeline service triggered by Git pushes. Exposes an HTTP API (Axum), manages a job queue backed by SQLite, and runs a background worker that evaluates Nix flakes, builds closures, and publishes results to the binary cache.

## Why

Procurator's GitOps model requires that every git commit produces a set of Nix closures (immutable VM images). This service is the bridge between "code was pushed" and "VMs are ready to deploy." It evaluates the Nix flake to determine what changed, builds only the affected closures, pushes them to the cache, and (eventually) notifies the control plane.

## How It Works

```
git push → post-receive hook → CI API → job queue (SQLite) → worker thread
  → nix eval → nix build → push to cache → notify control plane
```

## Status

Scaffolded — the job queue, database, HTTP API, and worker loop are wired together. Core Nix eval/build logic is in progress.
