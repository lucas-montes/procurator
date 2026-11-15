# Context Map

Quick navigation for AI sessions working on Procurator.

## Core Context Files
- [overview.md](overview.md) - Project overview and current state
- [glossary.md](glossary.md) - Terminology and definitions
- [architecture.md](architecture.md) - System architecture (TODO)
- [patterns.md](patterns.md) - Coding patterns and conventions (TODO)

## Domain-Specific Context
- [control-plane/](control-plane/) - Control plane pull-assignment node state
- [worker/](worker/) - Worker pull reconciliation + public proxy
- [vcs/](vcs/) - VCS/version control system (git porcelain)
- [stack/](stack/) - Stack CLI module layout and behavior
  - [stack/lifecycle.md](stack/lifecycle.md) — Service lifecycle monitoring and restart
  - [stack/healthcheck.md](stack/healthcheck.md) — Healthcheck system (schema + runner)
- [plans/](plans/) - Implementation plans and task tracking
- [specs/](specs/) - Technical specifications (e.g. Nix schema)
  - [specs/ci-service.md](specs/ci-service.md) — CI service target design: flake checks, per-check logs, cache reuse, `procurator.nix` config, ports, security
- [tmp/](tmp/) - Session scratch space

## Codebase Map

### CLI (`cli/`)
- `cli/pcr/main.rs` - CLI entry point with clap derive
- `cli/pcr/vcs/cli.rs` - VCS commands (project/repo/agent operations)
- `cli/pcr/stack/` - Stack management commands
  - `cli/pcr/stack/cli.rs` - CLI dispatch (start only)
  - `cli/pcr/stack/config.rs` - Flake config types, parsing, validation
  - `cli/pcr/stack/health.rs` - Healthcheck runner (parse_test, run_healthcheck)
  - `cli/pcr/stack/service.rs` - Type-state services, manifest, supervisor, healthcheck integration
  - `cli/pcr/stack/logging.rs` - Log writers (terminal, file, both)
  - `cli/pcr/stack/watch.rs` - File watcher for hot-reload
- `cli/pcr/agents/` - Workspace lifecycle command stubs (`up`/`down`/`stop`/`start`/`restart`; placeholders only). Real agent workspace prep (clone + branch) lives in `vcs/cli.rs`'s `agent` subcommand, not here.

### Git Utilities (`repo_outils/`)
- `repo_outils/src/git/repo.rs` - GitRepo operations plus `RepoCache` local mirror acceleration
- `repo_outils/src/git/process.rs` - Process-oriented repository path helpers
- `repo_outils/src/nix/` - Nix-related utilities

### Other Crates
- `autonix/` - Nix automation and analysis
- `repohub/` - Repository hub service
- `control_plane/` - Coordination service
- `worker/` - Background job execution
- `ci_service/` - CI service, hexagonal like `cache` (spec: `specs/ci-service.md`)
  - `ci_service/src/domain/run.rs` - run identity, statuses, lifecycle transitions
  - `ci_service/src/application/` - `Service` use cases and `ports.rs` (`RunStore`, `Clock`)
  - `ci_service/src/adapters/` - `sqlite.rs`, `memory.rs` (held to `store_contract.rs`), `system.rs`, `http/`

### Cache (`cache/`)
Embeddable artifact cache in a hexagonal layout: named public/private caches, token auth, Nix front-end only. Plain HTTP, no TLS.
- `cache/src/domain/` - Pure rules: `cache.rs` (names, visibility, spec), `digest.rs` (SHA-256, nix32), `access.rs` (principals, grants, authorize), `nix/` (narinfo, NAR file names, signing)
- `cache/src/application/` - `Service` with every use case (`nix.rs`, `tokens.rs`, `gc.rs`) and `ports.rs` (`BlobStore`, `Index`, `Clock`, `Entropy`)
- `cache/src/adapters/` - `http/` (axum router: Nix routes under `/<cache>/`, token API under `/_api/v1/tokens`), `fs.rs` (disk blobs), `sqlite.rs` (index), `memory.rs` (in-memory blobs and index), `system.rs` (clock, randomness)
- `cache/src/config.rs`, `cache/src/main.rs` - TOML config and the composition root
- `cache/tests/` - `http.rs` (in-process router tests), `nix_e2e.rs` (real `nix copy`, `--ignored`)

## Current Work
- **Pending plan:** `plans/control-plane-web-console.md` (T01–T19 pending; separate `console` service that builds a live map of machines/VMs/units and real-time messages purely by parsing each service's JSON `tracing` logs (file / journal-gatewayd / journalctl sources), plus a control board using capnp for actions and machine/unit/VM pages; log convention in `specs/log-events.md`)
- **Completed plan:** `plans/cache-hexagonal-rewrite.md` (T01–T14 done; cache rewritten as a hexagonal library with named public/private caches and token auth)
- **Pending plan:** `plans/rust-log-env-var.md` (T01 done, T02 pending)
- **Completed plan:** `plans/control-plane-pull-assignments.md` (T01–T09 done; control-plane pull loop, worker poll/reconcile/heartbeat, Nix inventory, cross-crate e2e)
- **Completed plan:** `plans/healthcheck.md` (T01–T05 done)
- **Completed plan:** `plans/orchestrator-lifecycle.md` (T01–T04 done)
- **Completed plan:** `plans/source-file-watch.md` (T01–T05 done)
- **Completed plan:** `plans/hot-reload-watch-mode.md` (T01–T06 done)
- **Completed plan:** `plans/stack-refactor-implementation.md` (T01–T06 done, committed as 2d95d21)
- **Completed plan:** `plans/stack-lifecycle-improvement.md` (T01–T08 done, committed as 67e600c)
