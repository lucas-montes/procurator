# Repo Outils — Git & Nix Utilities

## What

A shared utility library for Git repository management and Nix operations. Provides a `GitRepo` abstraction for bare repositories with path/URL management, SSH URL generation, and Nix-compatible `git+file://` URLs. Also includes Nix-specific helpers (flake evaluation, build log parsing).

## Why

Both `repohub` (web UI) and `ci_service` (build pipeline) need to create, clone, and manage Git repositories and invoke Nix commands. This crate extracts that shared plumbing so neither service reimplements git/nix integration logic.

## Key Types

- **`GitRepo`** — Represents a bare Git repository on disk. Handles creation, SSH URL generation, and Nix-compatible URL formatting.
- **`nix::*`** — Wrappers around Nix CLI commands (evaluate, build, log parsing).
