# Plan: VCS Multi-Repo Porcelain (Latest)

## Change Summary

Build a git porcelain with explicit namespaces for multi-repo and single-repo workflows:
- `pcr vcs project` for projects with submodules
- `pcr vcs repo` for single repositories
- `pcr vcs agent` for agent workspace management

This consolidated plan merges all prior VCS plans and preserves the most recent implementation decisions and completion status.

## Canonical Decisions

- Explicit namespaces only (no smart mode detection).
- Selective project targeting via `--repos` and `--exclude`.
- Project = Git repository with submodules.
- Nix cache pull is automatic via `extra-substituters`; push is optional and resolved from `flake.nix`.
- Agent workspace path is co-located with the project at `<project-dir>/agents/<branch>/`.
- Local clone acceleration uses bare mirrors at `~/.cache/procurator/repo-cache/` plus `--reference`.
- SSH keys are the unified auth/signing approach for Git + Nix cache operations.

## Success Criteria

1. `pcr vcs project clone <url>` clones project and submodules.
2. `pcr vcs project pull --repos docs,config` updates only selected repos.
3. `pcr vcs project push` pushes main repo first, then changed submodules only.
4. `pcr vcs repo clone <url>` clones a single repo.
5. `pcr vcs repo pull` and `pcr vcs repo push` work with optional Nix cache integration.
6. `pcr vcs repo branch <name>` creates/checks out branch via git2.
7. `pcr vcs project branch <name>` branches main + all submodules.
8. `pcr vcs agent prepare <url> <branch>` creates workspace in `<project-dir>/agents/<branch>/`.
9. `pcr vcs agent list` shows active workspaces.
10. Local repo cache speeds up clone operations via `--reference`.
11. Command execution logs include timing and completion details.

## Task Stack

- [x] T01: Implement VCS + Agent CLIs with two namespaces (stubs)
- [x] T03: Implement clone, pull, push with submodule support
- [x] T04: Add Nix cache push to repo commands
- [x] T05: Implement branch for repo (git2)
- [x] T06: Implement smart project push (changed submodules only)
- [x] T07: Implement agent prepare and list
- [x] T08: Add local bare repo cache
- [x] T09: Add execution logs and timing coverage

## Next Task (Ready)

- **Next task:** None — plan task stack complete.
- **ready_for_implementation:** yes

## T08 Scope

- Add `RepoCache` for bare mirrors under `~/.cache/procurator/repo-cache/`.
- Initialize mirrors with `git clone --mirror`.
- Refresh mirrors with `git fetch --all`.
- Use `--reference <mirror>` during clone paths where applicable.
- Reuse existing SSH auth path from current Git operations.

### T08 Done When

- Clone paths automatically use cache when mirror exists.
- Cache is created/updated successfully for remote repos.
- Behavior gracefully falls back to normal clone when cache is unavailable.

### T08 Verification

- Clone the same repository twice and verify second clone uses reference mirror.
- Verify mirror directory creation/update under `~/.cache/procurator/repo-cache/`.
- Confirm no regressions in existing project/repo clone flows.

## Constraints

- Keep implementation porcelain-level and explicit.
- Keep changes minimal and in existing VCS/repo utility modules.
- Do not rework command UX or namespace structure.

## Assumptions

1. Git is available in PATH.
2. Nix is installed where cache operations are expected.
3. SSH keys/agent are configured for Git and cache endpoints.
4. Submodule URLs are reachable with configured credentials.
