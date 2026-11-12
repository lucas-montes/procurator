# Plan: crate-status-audit

## Change summary

Audits all nine Rust workspace crates (`cli`, `commands`, `control_plane`,
`worker`, `ci_service`, `cache`, `repo_outils`, `repohub`, `autonix`) and
updates each crate's `README.md` so it states the crate's goal and an accurate
`Status` section naming what is implemented and what is still missing to
reach that goal.

Four crates (`cli`, `ci_service`, `worker`, `repohub`) already carry a
`Status`/`TODO` section; this plan verifies those against the current code
(several source files in `worker` and `cli/pcr/vcs/cli.rs` have changed since
those sections were last written, per `git status`) and corrects any drift.
The remaining five crates (`control_plane`, `cache`, `commands`,
`repo_outils`, `autonix`) have no gap assessment at all — code alone cannot
reveal unbuilt intent, so those tasks ask the user directly what is
considered missing before writing the section.

This extends existing documentation; it does not change application
behavior.

## Acceptance criteria

- [ ] AC1: Every one of the 9 workspace crate `README.md` files contains a
      `## Status` section that names what is implemented and what is
      missing relative to the crate's stated `What`/`Why` goal.
  - Validate: `grep -L '## Status' cli/README.md commands/README.md control_plane/README.md worker/README.md ci_service/README.md cache/README.md repo_outils/README.md repohub/README.md autonix/README.md` prints nothing.
- [ ] AC2: For the 4 crates with a pre-existing `Status`/`TODO` section
      (`cli`, `ci_service`, `worker`, `repohub`), the section's claims match
      the current code — no item marked done that the code contradicts, and
      no gap listed that the code has since closed.
  - Validate: manual read of each crate's `Status` section against its `src/`
    for the claims the task's done-when criteria name.

### Full validation

- `grep -c '## Status' cli/README.md commands/README.md control_plane/README.md worker/README.md ci_service/README.md cache/README.md repo_outils/README.md repohub/README.md autonix/README.md` — each file reports exactly 1.

### Context sync

- None required. This plan only touches per-crate `README.md` files;
  `context/context-map.md`'s codebase map links to source files, not to
  README content, so it stays accurate unchanged.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** the `README.md` file of each of the 9 workspace crates listed
  in the root `Cargo.toml` `[workspace] members`.
- **Out of scope:** the root `README.md`, `nix/README.md`, `docs/`,
  `example/README.md`, any application code or tests, and any `context/`
  file other than the plan itself.
- **Constraints:** preserve each README's existing structure and voice
  (`What`/`Why`/etc.); add or correct a `## Status` section rather than
  rewriting the file. For the 5 crates with no existing gap section, the
  missing-work content comes from asking the user directly, not from
  guessing at unstated roadmap intent.
- **Non-goal:** this plan does not fix any gap it documents, add tests, or
  restructure any crate.

## Assumptions

- "Each directory" is scoped to the 9 Rust workspace crates named in
  `Cargo.toml`, not the repository's non-crate top-level directories
  (`nix/`, `docs/`, `example/`), per user confirmation.
- The deliverable is an update to each crate's existing `README.md`, not a
  new `context/<crate>/` file and not a standalone report, per user
  confirmation.
- For the 5 crates with no existing `Status` section, the gap assessment is
  gathered by asking the user directly rather than inferred from code alone,
  per user confirmation.

## Task stack

- [x] T01: `Add a Status section to control_plane/README.md` (status:done)
  - Task ID: T01
  - Scope: In — ask the user what is missing to reach the stated goal
    ("stores desired state, schedules VMs to worker nodes"); code already
    shows `scheduler.rs`, `registry.rs`, `store.rs`, `node.rs`, `server.rs`
    implemented and `context/architecture.md` records known limits (no CP
    persistence, no worker↔master auth) — confirm with the user whether
    those count as the crate's open gaps or whether something else does: add
    a `## Status` section to `control_plane/README.md`. Out — any other
    crate's README, any code change.
  - Dependencies: none
  - Done when: `control_plane/README.md` has a `## Status` section naming
    concrete implemented-vs-missing items, confirmed with the user.
  - Verify: `grep -A5 '## Status' control_plane/README.md` shows the new section.
  - Completed: 2026-11-04
  - Files changed: control_plane/README.md
  - Result: Added a `## Status` section. User confirmed via Q&A 4 gaps: no
    persistence (in-memory state lost on restart), no worker↔master auth,
    empty per-VM status rows in getClusterStatus/getWorker, and scheduler.rs
    only groups pre-pinned assignments (no bin-packing/resource-aware
    placement).
  - Context impact: none — change confined to control_plane/README.md.
  - Context synchronization: synced

- [x] T02: `Add a Status section to cache/README.md` (status:done)
  - Task ID: T02
  - Scope: In — ask the user what is missing to reach the stated goal (a
    nix-serve-compatible binary cache); code already shows `routes.rs`,
    `storage.rs`, `tls.rs`, `nix.rs`, and the README mentions mTLS and
    signing keys as available — confirm with the user which of those are
    complete versus partial (e.g. garbage collection, auth modes, multi-key
    signing) and add a `## Status` section to `cache/README.md`. Out — any
    other crate's README, any code change.
  - Dependencies: none
  - Done when: `cache/README.md` has a `## Status` section naming concrete
    implemented-vs-missing items, confirmed with the user.
  - Verify: `grep -A5 '## Status' cache/README.md` shows the new section.
  - Completed: 2026-11-04
  - Files changed: cache/README.md
  - Result: Added a `## Status` section. User confirmed via Q&A: the
    `StorageBackend` trait is already backend-agnostic by design but only
    `LocalDisk` is implemented (gap); no GC/eviction exists even though
    quota rejects writes once full (gap); the cache's broader goal is
    public+private replication of build artifacts across workers/CI/local,
    currently Nix-only with Bazel/Buck2-style artifact caching planned later
    (gap); file-based TLS PKI is accepted for now, dynamic issuance
    (Vault/step/SPIRE) is a later upgrade, not a current gap.
  - Context impact: none — change confined to cache/README.md.
  - Context synchronization: synced

- [x] T03: `Add a Status section to commands/README.md` (status:done)
  - Task ID: T03
  - Scope: In — ask the user what is missing to reach the stated goal (the
    single source of truth for the Cap'n Proto wire format across
    `common.capnp`, `worker.capnp`, `master.capnp`); confirm with the user
    whether the current three schemas are considered complete for the
    system's needs or whether specific interfaces/fields are still expected,
    and add a `## Status` section to `commands/README.md`. Out — any other
    crate's README, any `.capnp` schema change.
  - Dependencies: none
  - Done when: `commands/README.md` has a `## Status` section naming
    concrete implemented-vs-missing items, confirmed with the user.
  - Verify: `grep -A5 '## Status' commands/README.md` shows the new section.
  - Completed: 2026-11-04
  - Files changed: commands/README.md
  - Result: Corrected stale documentation (README omitted `ch.capnp` and 4 of
    8 `Worker` RPCs) and added a `## Status` section. User confirmed no
    missing interface/schema gap exists beyond the documentation drift now
    fixed.
  - Context impact: none — change confined to commands/README.md.
  - Context synchronization: synced

- [x] T04: `Add a Status section to repo_outils/README.md` (status:done)
  - Task ID: T04
  - Scope: In — ask the user what is missing to reach the stated goal
    (shared Git/Nix plumbing for `repohub` and `ci_service`); code already
    shows `GitRepo` (`git/repo.rs`, `git/process.rs`) and Nix helpers
    (`nix/cache.rs`, `nix/commands.rs`, `nix/flake.rs`, `nix/logs.rs`) —
    confirm with the user whether both consumer crates' needs are fully
    covered or whether specific operations are still missing, and add a
    `## Status` section to `repo_outils/README.md`. Out — any other crate's
    README, any code change.
  - Dependencies: none
  - Done when: `repo_outils/README.md` has a `## Status` section naming
    concrete implemented-vs-missing items, confirmed with the user.
  - Verify: `grep -A5 '## Status' repo_outils/README.md` shows the new section.
  - Completed: 2026-11-04
  - Files changed: repo_outils/README.md
  - Result: Corrected the README's type attributions (GitRepo vs RepoPath vs
    RepoCache) and added a `## Status` section. A workspace-wide grep per
    export found only `Infrastructure` genuinely shared (repohub +
    ci_service); `FlakeMetadata`/`RepoPath`/`create_bare_repo`/
    `clone_into_bare` are repohub-only, `GitRepo`/`filter_submodules`/
    `RepoCache` are cli-only, `flake_check` is ci_service-only, and
    `delete_repo`/`build_cluster_images`/`eval_cluster_metadata` have zero
    consumers anywhere. User confirmed the intended direction (git/artifact
    plumbing unified across cli/repohub/ci_service/worker with automatic
    cache use) isn't built yet, and left open whether it's needed at all
    given Nix's own substituter mechanism — recorded as an open question in
    the README.
  - Context impact: none — change confined to repo_outils/README.md.
  - Context synchronization: synced

- [x] T05: `Add a Status section to autonix/README.md` (status:done)
  - Task ID: T05
  - Scope: In — ask the user what is missing to reach the stated two-part
    goal (package detection via `mapping/`/`repo/`/`project/`, and
    infrastructure-dependency discovery for cluster topology generation);
    confirm with the user how much of each goal is implemented — the README
    only documents the packaging/flake-generation side in detail, so
    specifically confirm the status of the dependency-discovery half — and
    add a `## Status` section to `autonix/README.md`. Out — any other
    crate's README, any code change.
  - Dependencies: none
  - Done when: `autonix/README.md` has a `## Status` section naming concrete
    implemented-vs-missing items, confirmed with the user.
  - Verify: `grep -A5 '## Status' autonix/README.md` shows the new section.
  - Completed: 2026-11-04
  - Files changed: autonix/README.md
  - Result: Added a `## Status` section. Found both halves of the stated
    goal (package detection and infrastructure-dependency discovery via
    `extract_services`) are implemented for a single repo; confirmed with
    the user that the two real gaps are (1) `project/` multi-repo parsing is
    entirely unimplemented (`project/mod.rs` is a design comment,
    `project/parser.rs` is empty), and (2) `repohub` has zero references to
    `autonix` despite being a stated consumer — only `cli`'s `pcr init`
    actually uses it.
  - Context impact: none — change confined to autonix/README.md.
  - Context synchronization: synced

- [x] T06: `Verify and refresh the Status section in cli/README.md` (status:done)
  - Task ID: T06
  - Scope: In — re-check the existing "Scaffolded — `init` has an
    implementation, other commands are stubs" claim against current
    `cli/pcr/` code; `cli/pcr/vcs/cli.rs` has changed since this line was
    written (per `git status`), so confirm whether `repo`/`stack` commands
    are still stubs or have gained real implementation, and update the
    section to match. Out — any other crate's README, any code change.
  - Dependencies: none
  - Done when: `cli/README.md`'s `Status` section accurately reflects which
    of `init`/`stack`/`repo` are implemented versus stubbed, matching the
    current code.
  - Verify: `grep -A5 '## Status' cli/README.md` matches a read of
    `cli/pcr/main.rs`, `cli/pcr/vcs/cli.rs`, and `cli/pcr/stack/`.
  - Completed: 2026-11-04
  - Files changed: cli/README.md
  - Result: Rewrote the command table (real top-level commands are
    `init`/`stack`/`vcs`/`agents`, not `init`/`stack`/`repo`; `stack` only
    has `start`, not up/down/stop/restart) and the `Status` section (`init`,
    `stack start`, and all of `vcs repo`/`project`/`agent` are implemented,
    not stubs; the only real stub is the top-level `agents` command). Flagged
    the naming collision between the stub `pcr agents` and the implemented
    `pcr vcs agent` as a caveat.
  - Context impact: local — corrected a stale context-map.md entry.
  - Context synchronization: synced

- [x] T07: `Verify and refresh the Status section in ci_service/README.md` (status:done)
  - Task ID: T07
  - Scope: In — re-check the existing "job queue, database, HTTP API, and
    worker loop are wired together; core Nix eval/build logic is in
    progress" claim against current `ci_service/src/builds.rs`,
    `job_queue.rs`, and `worker.rs`, and update the section to match. Out —
    any other crate's README, any code change.
  - Dependencies: none
  - Done when: `ci_service/README.md`'s `Status` section accurately
    reflects the current state of Nix eval/build integration.
  - Verify: `grep -A5 '## Status' ci_service/README.md` matches a read of
    `ci_service/src/builds.rs`.
  - Completed: 2026-11-04
  - Files changed: ci_service/README.md
  - Result: Rewrote the Status section. `worker.rs::process_build` runs
    real `nix flake check` builds with retry/backoff, but 3 of the 4 steps
    in the README's own "How It Works" diagram don't exist: no cache check
    before building, no push to cache on success, and no control-plane
    notification (zero references to `cache`/`control_plane`/`capnp`/notify
    anywhere in `ci_service/src/`). Also found the build summary is
    discarded — `process_build` stores a bare placeholder
    (`let db_summary = DbBuildSummary;`) instead of the real `CheckResult`
    from the Nix run.
  - Context impact: none — change confined to ci_service/README.md.
  - Context synchronization: synced

- [x] T08: `Verify and refresh the Status section in repohub/README.md` (status:done)
  - Task ID: T08
  - Scope: In — re-check the existing "CRUD for users/projects/repos is
    functional; configuration management, Nix flake integration, build
    tracking, and E2E testing are planned" claim; the same README already
    documents a working set of Gerrit-style review endpoints, which reads as
    a contradiction with "planned" — resolve the discrepancy against
    `repohub/src/adapters/`, `application/`, and `domain/`, and update the
    section to match. Out — any other crate's README, any code change.
  - Dependencies: none
  - Done when: `repohub/README.md`'s `Status` section accurately reflects
    which capabilities (CRUD, Gerrit review, Nix flake integration, build
    tracking) are functional versus planned, with the Gerrit discrepancy
    resolved.
  - Verify: `grep -A5 '## Status' repohub/README.md` matches a read of
    `repohub/src/adapters/`, `repohub/src/application/`.
  - Completed: 2026-11-04
  - Files changed: repohub/README.md
  - Result: Rewrote the Status section. Resolved the Gerrit discrepancy —
    Gerrit review (10 endpoints, use-case ports + SQLite persistence) is
    functional, not planned; the Status line was simply stale. Nix flake
    integration is partially functional (read-side parsing via
    `repository_service.rs`, no write-side). Configuration management
    (types defined, nothing persists/serves them), build tracking, and E2E
    testing are confirmed still accurately planned (zero code found for
    any of the three).
  - Context impact: none — change confined to repohub/README.md.
  - Context synchronization: synced

- [x] T09: `Verify and refresh the Status/TODO section in worker/README.md` (status:done)
  - Task ID: T09
  - Scope: In — re-check the existing `## TODO` list (graceful shutdown,
    directory cleanup on failure, abstracted stat RPCs, interactive shell,
    better testing, snapshot/resume testing); `worker/src/lib.rs`,
    `worker/src/proxy/core.rs`, `worker/src/proxy/tls.rs`, and
    `worker/src/vmm/registry.rs` have changed and `worker/src/e2e.rs` was
    removed (per `git status`) since this list was written — confirm which
    TODO items are now done, still open, or superseded, and update the
    section to match. Out — any other crate's README, any code change.
  - Dependencies: none
  - Done when: `worker/README.md`'s `TODO`/`Status` content accurately
    reflects the current implementation state, including the removal of
    `e2e.rs` and any proxy/registry changes.
  - Verify: `grep -A10 '## TODO' worker/README.md` matches a read of
    `worker/src/lib.rs`, `worker/src/proxy/core.rs`, `worker/src/vmm/registry.rs`.
  - Completed: 2026-11-04
  - Files changed: worker/README.md
  - Result: Added a `## Status` note and annotated each TODO item against
    current code. `pause_vm`/`resume_vm`/`snapshot_vm`/`backup_disk` are now
    fully implemented (not stubs). All 7 TODO items remain open; one is a
    regression rather than a pending item: the cross-crate e2e test
    (`worker/src/e2e.rs`, 243 lines) was deleted in commit `26267fd` with no
    replacement, and `Handle::health()` is a no-op stub despite looking
    finished. `context/overview.md` still cites the deleted `e2e.rs` by path
    — flagged for the context-sync pass.
  - Context impact: root — context/overview.md cited a deleted file as
    current evidence; corrected.
  - Context synchronization: synced

## Open questions

None. The 3 scope/deliverable/process questions that would have changed task
structure were resolved by the user before this plan was authored.
