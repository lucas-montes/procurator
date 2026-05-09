# Procurator Overview

Declarative reproducible developer platform powered by Nix.

## Core Purpose
Procurator provides a declarative way to manage development environments and project stacks using Nix. The CLI tool `pcr` manages projects, repositories, and build environments.

## Key Components
- **CLI (`cli/`)**: Command-line interface with subcommands for VCS, stack management, and agent workspaces
- **repo_outils (`repo_outils/`)**: Git and Nix utility functions used across the project
- **autonix (`autonix/`)**: Nix-related automation and analysis tools
- **repohub (`repohub/`)**: Repository hub service for project discovery
- **control_plane (`control_plane/`)**: Central coordination service
- **worker (`worker/`)**: Background job execution service
- **ci_service (`ci_service/`)**: Continuous integration service
- **cache (`cache/`)**: Caching service for build artifacts

## CLI Namespaces (pcr)
- `pcr vcs`: Version control operations (project/repo management)
- `pcr stack`: Local project stack lifecycle
- `pcr agents`: Workspace management for AI agents
- `pcr init`: Initialize workspace

## Technology Stack
- **Language**: Rust (edition 2024)
- **Build tool**: Cargo with workspace structure
- **Git operations**: git2 (Rust bindings with vendored libgit2)
- **Nix integration**: Flake-based configurations
- **Async runtime**: Tokio
- **CLI framework**: Clap with derive macros

## Current State (as of T10)
- VCS commands are implemented for `pcr vcs repo`, `pcr vcs project`, and `pcr vcs agent`.
- Project operations support submodules with selective `--repos` / `--exclude` filtering.
- Repo push supports optional Nix cache upload when cache URL is configured in flake settings.
- Branch operations exist for repo and project flows (`project branch` remains a guided/manual stub path).
- Agent workspace prepare/list commands are implemented with co-located workspaces at `<project-dir>/agents/<branch>/`.
- Local clone acceleration is enabled through bare mirror cache references at `~/.cache/procurator/repo-cache/`.
- VCS command handlers emit execution timing logs (e.g. `... completed in ...`) for operational visibility.
