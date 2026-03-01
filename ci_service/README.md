# CI Service — Build Pipeline

## What

A CI/CD pipeline library (with optional HTTP binary) triggered by Git pushes. Manages a job queue backed by SQLite, evaluates Nix flakes, builds closures, and publishes results to the binary cache.

## Why

Procurator’s GitOps model requires that every git commit produces a set of Nix closures. This service bridges "code was pushed" and "VMs are ready to deploy." It pulls from the cache first (closures may already exist from a `pcr push`), builds only on cache miss, and notifies the control plane.

## How It Works

```
git push → post-receive hook → CI job queue (SQLite) → worker thread
  → check cache → nix eval/build (on miss) → push to cache → notify control plane
```

## Library-First Design

Structured as a library (`ci_service::*`) with a thin binary (`main.rs`). The library can be embedded directly into a monolith alongside repohub, or run as a standalone service with its own HTTP API. The `web` feature gate controls whether the Axum routes are compiled.

## Status

Scaffolded — the job queue, database, HTTP API, and worker loop are wired together. Core Nix eval/build logic is in progress.
