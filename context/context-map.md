# Context Map

Quick navigation for AI sessions working on Procurator.

## Core Context Files
- [overview.md](overview.md) - Project overview and current state
- [glossary.md](glossary.md) - Terminology and definitions
- [architecture.md](architecture.md) - System architecture (TODO)
- [patterns.md](patterns.md) - Coding patterns and conventions (TODO)

## Domain-Specific Context
- [vcs/](vcs/) - VCS/version control system (git porcelain)
- [plans/](plans/) - Implementation plans and task tracking
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
- **Active plan**: `plans/vcs-latest.md`
- **Completed**: T01, T03, T04, T05, T06, T07, T08, T09
- **Next**: T10 (final validation and cleanup)
