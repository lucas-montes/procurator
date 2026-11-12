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

Scaffolded — the job queue, database, HTTP API, and worker loop are wired together, and the worker runs real `nix flake check` builds (`worker.rs::process_build` → `repo_outils::nix::flake_check`) with retry/backoff on failure.

Three of the four pipeline steps in "How It Works" above don't exist yet, independent of the `nix flake check` call itself:

- **No cache check before building** — the worker always runs the full `nix flake check`; there's no lookup against the binary cache to skip an already-built commit.
- **No push to cache on success.**
- **No control-plane notification** — nothing in `ci_service/src/` references `control_plane`, `capnp`, or any notify path; there's no `cache` reference either. A successful build updates the job's own status in SQLite and stops there.
- **Build summary is a placeholder** — on success, `process_build` stores `let db_summary = DbBuildSummary;`, a bare default; the real `CheckResult`/summary data from the Nix run (steps, timing, pass/fail detail) is computed but discarded, not persisted. This is also why the Notes item below (track which tests failed) isn't possible yet — there's no structured summary to look at.

## Notes
An important features would be to save the runs, and keep track of what tests failed so we can identify flaky tests and rerun them or notify the user
