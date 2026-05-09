# VCS - Version Control System

Procurator's git porcelain for managing single repos and multi-repo projects with submodules.

## Namespaces

### `pcr vcs repo` (Single Repository)
Commands for operating on a single git repository.

| Command | Description | Implementation |
|---------|-------------|----------------|
| `clone <url>` | Clone a single repository | Cache-aware clone: local mirror `--reference` + git2 fallback |
| `pull` | Pull latest changes (fetch + merge) | git2 fetch + merge with conflict detection; refreshes local mirror cache |
| `push` | Push local commits | git2 push with ssh-agent auth |
| `branch <name>` | Create a new branch | Implemented via `GitRepo::branch()` (git2 reference + set HEAD) |

### `pcr vcs project` (Multi-Repo Project)
Commands for operating on projects with submodules.

| Command | Description | Implementation |
|---------|-------------|----------------|
| `list` | List available projects | Checks `~/.config/procurator/projects.toml` |
| `clone <url>` | Clone project with submodules | git2 clone + init_submodules() |
| `pull [--repos X] [--exclude Y]` | Pull with selective submodules | git2 fetch + merge + filter_submodules() |
| `push [--repos X] [--exclude Y]` | Push with selective submodules | git2 push + filter_submodules() |
| `branch <name>` | Create branch across all repos | Guided/manual stub in CLI output (full automation pending) |

## Key Implementation Details

### Git2 Operations Module (`repo_outils/src/git/repo.rs`)
- **GitRepo struct**: Wraps `git2::Repository` with private fields (`repo`, `path`)
- **Authentication**: Uses ssh-agent via `git2::Cred::ssh_key_from_agent()`
- **Clone**: `GitRepo::clone()` - tries local `RepoCache` mirror + `--reference`, falls back to direct git2 clone
- **Pull**: `GitRepo::pull()` - fetch + merge with conflict detection and cleanup; attempts mirror refresh from origin URL
- **Push**: `GitRepo::push()` - push with remote callbacks for auth
- **Branch**: `GitRepo::branch()` - create branch using `repo.reference()` and move HEAD to new branch
- **Submodules**: `init_submodules()` and `list_submodules()` methods
- **RepoCache**: `RepoCache` manages bare mirrors in `~/.cache/procurator/repo-cache/` (`clone --mirror`, `fetch --all`, `clone --reference`)
- **Error handling**: `Git2Error` enum includes `GitError`, `AuthError`, `IoError`, and `CommandError` variants

### SubmoduleInfo Struct
Private fields with getter methods:
- `name()` - Returns submodule name
- `path()` - Returns submodule path as `&Path`
- `url()` - Returns submodule URL

### Selective Repo Operations
The `filter_submodules()` function filters submodules based on:
- `--repos X,Y` (include only specified)
- `--exclude X,Y` (exclude specified)

Logic: Exclude filters applied first, then include filters. Uses `HashSet<&str>` for efficient lookup.

```rust
pub fn filter_submodules(
    submodules: Vec<SubmoduleInfo>,
    include: &Option<Vec<String>>,
    exclude: &Option<Vec<String>>,
) -> Vec<SubmoduleInfo>
```

## Configuration

### Project Registry (TODO)
- Config file: `~/.config/procurator/projects.toml`
- Format: TBD (parsing not yet implemented)
- Fallback: Clone directly from URL

### Nix Cache Operations Module (`repo_outils/src/nix/cache.rs`)
- **read_cache_url()**: Reads cache URL from `nix eval -f flake.nix nixConfig.extra-substituters --json`
- **push_to_cache()**: Pushes single derivation with `nix copy --to <url> <path>`
- **push_all_to_cache()**: Pushes all artifacts from `./result` symlink
- **pull_from_cache()**: Pulls derivation with `nix copy --from <url> <path>`
- **find_nix_artifacts()**: Finds Nix artifacts in current directory (`./result`, `result-*`)
- **Error handling**: `CacheError` enum with `Io`, `JsonParse`, `NixCommandFailed`, `NoCacheConfig`, `NoArtifacts`

## Dependencies
- **git2 = "0.19"** with `vendored-libgit2` feature (static linking)
- **dirs = "5.0"** for config directory detection

## Code Structure

### CLI (`cli/pcr/vcs/cli.rs`)
- Uses `Args` and `Subcommand` derive macros from clap
- `repo`, `project`, and `agent` command handlers are fully wired through `execute_*` functions
- **Repo branch**: Uses git2 `GitRepo::branch()` (no git CLI fallback for repo branch)
- **Execution timing**: command handlers log elapsed duration with `Instant::now()` + `completed in ...` logs

### Git Operations (`repo_outils/src/git/repo.rs`)
- Private fields with minimal public API
- Functions include clone/pull/push/branch/current_branch/has_changes and submodule helpers
- Clone path is cache-aware with graceful fallback to direct git2 clone

## Related Context
- [overview.md](../overview.md)
- [glossary.md](../glossary.md)
- [Plan: vcs-latest.md](../plans/vcs-latest.md)

### Nix Cache Operations Module (`repo_outils/src/nix/cache.rs`) - Added T04
- **read_cache_url()**: Reads cache URL from `nix eval -f flake.nix nixConfig.extra-substituters --json`
- **push_to_cache()**: Pushes single derivation with `nix copy --to <url> <path>`
- **push_all_to_cache()**: Pushes all artifacts from `./result` symlink
- **pull_from_cache()**: Pulls derivation with `nix copy --from <url> <path>`
- **find_nix_artifacts()**: Finds Nix artifacts in current directory (`./result`, `result-*`)
- **Error handling**: `CacheError` enum with `Io`, `JsonParse`, `NixCommandFailed`, `NoCacheConfig`, `NoArtifacts`
- **Signing**: Uses SSH keys (same as git operations) for `nix copy --to ssh://...`

### Integration in CLI (`cli/pcr/vcs/cli.rs`) - T04
- **Repo push**: After git push, optionally pushes Nix artifacts if cache URL found
- **Graceful skip**: If no cache URL or no `./result`, logs warning and continues
- **Signing**: Uses same SSH authentication as git operations

## Agent Commands

### `pcr vcs agent` (Agent Workspace Management)
Commands for preparing and listing agent workspaces.

| Command | Description | Implementation |
|---------|-------------|----------------|
| `prepare <url> <branch>` | Prepare workspace for an agent | git2 clone + branch creation |
| `list` | List active agent workspaces | Scan `agents/` directory |

### Workspace Structure
- **Path**: `<project-dir>/agents/<branch>/` (co-located with project)
- **Verification**: Error if workspace already exists
- **List output**: Table with Branch | Current | Status | Last Modified

### CLI Implementation (`cli/pcr/vcs/cli.rs`) - T07
- **AgentArgs**: Routes to Prepare or List subcommands
- **AgentPrepareArgs**: url, branch, optional name/path
- **execute_agent_prepare()**: 
  1. Derive project name from URL
  2. Create `<cwd>/agents/<branch>/` directory
  3. Error if exists
  4. Clone repo to workspace
  5. Create branch using `GitRepo::branch()`
- **execute_agent_list()**:
  1. Scan `<cwd>/agents/` for directories
  2. Open each as `GitRepo`
  3. Get current_branch() and has_changes()
  4. Print table sorted by last modified

### GitRepo Methods Added (T07)
- **current_branch()**: Returns current branch name using `head().shorthand()`
- **branch()**: Creates branch and sets HEAD using `set_head()` (not checkout_head)

## Related Context
- [overview.md](../overview.md)
- [glossary.md](../glossary.md)
- [Plan: vcs-latest.md](../plans/vcs-latest.md)
