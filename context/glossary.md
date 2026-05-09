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

## Git2 Library Terms
- **git2**: Rust bindings for libgit2 (used instead of shelling out to git command)
- **AnnotatedCommit**: A git commit with additional metadata (used in merge operations)
- **FetchOptions**: Configuration for git fetch operations (callbacks, credentials, etc.)
- **RemoteCallbacks**: Callback functions for remote operations (e.g., authentication)

## Nix Terms
- **Flake**: A Nix expression with inputs/outputs (defined in `flake.nix`)
- **Derivation**: A build action in Nix
- **Substituter**: A binary cache server for Nix store paths
