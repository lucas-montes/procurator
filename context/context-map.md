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
- [plans/](plans/) - Implementation plans and task tracking
- [specs/](specs/) - Technical specifications (e.g. Nix schema)
- [tmp/](tmp/) - Session scratch space

## Codebase Map

### CLI (`cli/`)
- `cli/pcr/main.rs` - CLI entry point with clap derive
- `cli/pcr/vcs/cli.rs` - VCS commands (project/repo/agent operations)
- `cli/pcr/stack/` - Stack management commands
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
- **Completed plan**: `plans/stack-refactor-implementation.md` (T01–T06 done, committed as 2d95d21)
- **Active plan**: `plans/stack-lifecycle-improvement.md` (T01–T07 done, T08 next)
