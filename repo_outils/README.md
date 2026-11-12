# Repo Outils — Git & Nix Utilities

## What

A shared utility library for Git repository management and Nix operations. Provides `GitRepo` (working-copy clone/pull/push/branch via `git2`) and `RepoPath`/`create_bare_repo`/`clone_into_bare` (bare-repo creation, path management, SSH URL generation, and Nix-compatible `git+file://` URLs). Also includes Nix-specific helpers (flake evaluation, build log parsing, cache push/pull).

## Why

The intent is a single place for git/Nix plumbing so no consumer — `cli` (local use), `repohub` (web UI), `ci_service` (build pipeline), or `worker` — reimplements it. In practice today, usage is narrower than that; see Status.

## Key Types

- **`GitRepo`** / **`RepoCache`** — `git2`-backed working-copy operations (clone/pull/push/branch/submodules) plus a local bare-mirror cache for faster clones.
- **`RepoPath`** / **`create_bare_repo`** / **`clone_into_bare`** / **`delete_repo`** — bare-repository path management, creation, and URL formatting (SSH, Nix `git+file://`).
- **`nix::*`** — Wrappers around Nix CLI commands (evaluate, build, log parsing, binary-cache push/pull).

## Status

Partially realized against the "shared plumbing" goal. Checked directly against actual call sites across the workspace:

- **`Infrastructure`** is genuinely shared — used by both `repohub` and `ci_service`, matching the intended "CLI locally, repohub on the web" design.
- **`FlakeMetadata`**, **`RepoPath`**, **`create_bare_repo`**, **`clone_into_bare`** are used only by `repohub` today. The git/bare-repo side is meant to also serve `ci_service`/`worker` for a unified, cache-aware way of pulling artifacts, but that isn't built yet.
- **`GitRepo`**, **`filter_submodules`**, **`RepoCache`** are used only by `cli` (working-copy clone/pull/push, local mirror acceleration). Same intended direction — also reachable from `repohub`/`ci_service`/`worker` — not yet realized.
- **`flake_check`** is used only by `ci_service`.
- `delete_repo`, `build_cluster_images`, and `eval_cluster_metadata` are public but have no consumer anywhere in the workspace.

Open question: whether a unified artifact-pulling/cache layer across `worker`/`ci_service`/`repohub` is actually needed, or whether Nix's own substituter mechanism (`nix copy`, `nix build` against a configured cache) already covers it without this abstraction. Not yet decided.
