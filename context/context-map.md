# Context Map

Quick navigation for AI sessions working on Procurator.

## Core Context Files
- [overview.md](overview.md) - Project overview and current state
- [glossary.md](glossary.md) - Terminology and definitions
- [architecture.md](architecture.md) - System architecture (TODO)
- [patterns.md](patterns.md) - Coding patterns and conventions (TODO)

## Domain-Specific Context
- [vcs/](vcs/) - VCS/version control system (git porcelain)
- [stack/](stack/) - Stack CLI module layout and behavior
  - [stack/lifecycle.md](stack/lifecycle.md) — Service lifecycle monitoring and restart (T02–T03)
- [plans/](plans/) - Implementation plans and task tracking
- [specs/](specs/) - Technical specifications (e.g. Nix schema)
- [tmp/](tmp/) - Session scratch space

## Codebase Map

### CLI (`cli/`)
- `cli/pcr/main.rs` - CLI entry point with clap derive
- `cli/pcr/vcs/cli.rs` - VCS commands (project/repo/agent operations)
- `cli/pcr/stack/` - Stack management commands
  - `cli/pcr/stack/cli.rs` - CLI dispatch (start only)
  - `cli/pcr/stack/config.rs` - Flake config types, parsing, validation
  - `cli/pcr/stack/service.rs` - Type-state services, manifest, supervisor
  - `cli/pcr/stack/logging.rs` - Log writers (terminal, file, both)
  - `cli/pcr/stack/watch.rs` - File watcher for hot-reload
- `cli/pcr/agents/` - Agent workspace commands

### Git Utilities (`repo_outils/`)
- `repo_outils/src/git/repo.rs` - GitRepo operations plus `RepoCache` local mirror acceleration
- `repo_outils/src/git/process.rs` - Process-oriented repository path helpers
- `repo_outils/src/nix/` - Nix-related utilities

### Other Crates
- `autonix/` - Nix automation and analysis
- `repohub/` - Repository hub service
- `control_plane/` - Coordination service
- `worker/` - Background job execution
- `ci_service/` - CI service
- `cache/` - Caching service

## Current Work
- **Active plan:** `plans/orchestrator-lifecycle.md` (T01–T03 done, T04 pending)
- **Active plan:** `plans/rust-log-env-var.md` (T01 done, T02 pending)
- **Completed plan:** `plans/source-file-watch.md` (T01–T05 done)
- **Completed plan:** `plans/hot-reload-watch-mode.md` (T01–T06 done)
- **Completed plan:** `plans/stack-refactor-implementation.md` (T01–T06 done, committed as 2d95d21)
- **Completed plan:** `plans/stack-lifecycle-improvement.md` (T01–T08 done, committed as 67e600c)
