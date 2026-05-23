# Glossary

## Git & VCS Terms
- **Repository (repo)**: A git repository, either standalone or as part of a project
- **Project**: A multi-repo git repository with submodules (main repo + submodules for docs, config, code, etc.)
- **Submodule**: A git repository embedded within another repository (managed via `.gitmodules`)
- **Clone**: Creating a local copy of a remote repository
- **Pull**: Fetching and merging remote changes (fetch + merge)
- **Push**: Uploading local commits to remote repository
- **Branch**: A parallel line of development in git

## Procurator-Specific Terms
- **pcr**: Procurator CLI binary name
- **VCS**: Version Control System - the git porcelain commands in `pcr vcs`
- **Project (pcr context)**: A multi-repo project managed by `pcr vcs project` commands
- **Repo (pcr context)**: A single repository managed by `pcr vcs repo` commands
- **Selective repos**: Using `--repos` flag to operate on specific submodules only
- **Exclude repos**: Using `--exclude` flag to skip certain submodules
- **Agent workspace**: A prepared directory for AI agent work (`<project-dir>/agents/<branch>/`) - co-located with project
- **Nix cache**: Binary cache for Nix derivations, configured in `flake.nix` via `nixConfig.extra-substituters`
- **Repo mirror cache**: Local bare mirror used to accelerate clones via `git clone --reference`; stored under `~/.cache/procurator/repo-cache/`.
- **Timing log**: Per-command duration log emitted after VCS command completion for runtime observability.
- **Normalized signal**: Forge-agnostic repohub event record (PR/review/commit/deployment/issue) used for metric computation.
- **Signal link key**: Correlation key (`PullRequest`, `CommitSha`, `Repository`) used to associate PR↔deploy↔incident events.
- **Failure signal**: Canonical mixed-model CFR/MTTR event projected from production deployments or incident issues.
- **Forge signal port**: Repohub application-layer contract (`ForgeSignalPort`) for retrieving normalized pull request/review/commit/deployment/issue signals independent of source forge.
- **Forge repository target**: Provider-neutral identifier (`repository_id`, `owner`, `name`) used by forge adapters to scope signal retrieval.
- **Weekly metric snapshot**: Persisted metric payload for a repository/week/version triple keyed by `(repository_id, week_start_utc, metric_version)`.
- **Weekly metric engine**: `WeeklyMetricEngine` contract that computes a deterministic 7-day snapshot from normalized PR/review/commit/deployment/issue signals.
- **Weekly window contract**: Fixed 7-day UTC window anchored at `week_start_utc` using half-open bounds `[start, end)`.
- **Failure episode (mixed model)**: First failure signal after a recovery/clean state across deployment and incident streams; used as CFR numerator events.
- **Nearest-forward recovery matching**: MTTR rule that pairs each in-window failure episode with the next recovery event in deterministic chronological/source-key order.
- **Signal dedup key**: Persistence uniqueness key for normalized event upserts: `(repository_id, signal_type, source_key)` where `source_key` uses source-native IDs/SHA.
- **DORA metrics API**: HTTP read endpoint at `/{username}/{project}/{repo}/dora/metrics` returning weekly DORA metric snapshots as JSON.
- **DORA background refresh**: Periodic tokio-based task that calls `RefreshOrchestrator::trigger_refresh` on a configurable interval (default 3600s).
- **DORA dashboard**: Read-only HTML page at `/{username}/{project}/{repo}/dora` rendering weekly DORA/productivity metrics via Askama template with grouped tables and Chart.js trends.

## Git2 Library Terms
- **git2**: Rust bindings for libgit2 (used instead of shelling out to git command)
- **AnnotatedCommit**: A git commit with additional metadata (used in merge operations)
- **FetchOptions**: Configuration for git fetch operations (callbacks, credentials, etc.)
- **RemoteCallbacks**: Callback functions for remote operations (e.g., authentication)

## Nix Terms
- **Flake**: A Nix expression with inputs/outputs (defined in `flake.nix`)
- **Derivation**: A build action in Nix
- **Substituter**: A binary cache server for Nix store paths
