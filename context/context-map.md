# Context Map

Quick navigation for AI sessions working on Procurator.

## Core Context Files
- [overview.md](overview.md) - Project overview and current state
- [glossary.md](glossary.md) - Terminology and definitions
- [architecture.md](architecture.md) - Repohub service architecture (DORA module, state wiring, background task)
- [patterns.md](patterns.md) - Coding patterns and conventions (TODO)

## Domain-Specific Context
- [vcs/](vcs/) - VCS/version control system (git porcelain)
- [repohub/](repohub/) - Repohub service domain notes
  - [repohub/normalized-signals.md](repohub/normalized-signals.md) - canonical normalized signal model (T03)
  - [repohub/forge-ports.md](repohub/forge-ports.md) - forge-agnostic ingestion contracts and adapter conformance (T04)
  - [repohub/weekly-metrics-engine.md](repohub/weekly-metrics-engine.md) - deterministic weekly computation contracts and edge-case semantics (T06)
  - [repohub/refresh-orchestrator.md](repohub/refresh-orchestrator.md) - async on-demand refresh pipeline: fetch→persist→compute→persist (T07)
  - [repohub/dora-api.md](repohub/dora-api.md) - DORA metrics HTTP API and periodic background refresh (T08)
  - [repohub/dora-dashboard.md](repohub/dora-dashboard.md) - DORA dashboard template, route, week picker, Chart.js trend charts, and formatting helpers (T09)
  - [repohub/github-oauth.md](repohub/github-oauth.md) - GitHub OAuth connection flow (config, redirect endpoint, CSRF nonce mechanism)
  - [repohub/github-oauth-testing.md](repohub/github-oauth-testing.md) - Manual test guide for OAuth flow and repo import
- [data-inventory.md](data-inventory.md) - GitHub signal field inventory and normalization readiness
- [repohub/persistence.md](repohub/persistence.md) - normalized signal + weekly snapshot persistence model (T05)
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
  - `repohub/src/application/ports.rs` - forge-agnostic signal ingestion contracts (`ForgeSignalPort`, `ForgeRepositoryTarget`, `ForgeError`)
  - `repohub/src/domain/signals/` - normalized signal model, linking, mixed failure/recovery projection, GitHub normalization transforms
- `control_plane/` - Coordination service
- `worker/` - Background job execution
- `ci_service/` - CI service
- `cache/` - Caching service

## Current Work
- **Completed plan**: `plans/github-oauth-connection.md` — all 7 tasks done
- **T07**: Validation passed — `cargo test -p repohub` 32/32, `cargo fmt --all -- --check` clean
- **Delivered**: GitHub OAuth connection flow, profile UI, repo list API, repo import UI, README docs
