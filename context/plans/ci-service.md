# Plan: ci-service

## Change summary

Rebuilds `ci_service` from a broken scaffold into a hexagonal service
(`domain` / `application` / `adapters`, like `cache`) that runs every flake
check of each push, reuses the binary cache, stores and streams per-check
logs, pushes what it built, notifies webhooks and deploys passing runs to the
control plane. The design and every decision is in
[`../specs/ci-service.md`](../specs/ci-service.md); this plan tracks the build
order of that spec (steps 1–10) plus the gaps the finished steps left.

Workflow: one step at a time; each step is committed after the user approves
it.

## Acceptance criteria

Numbering follows the spec. Status as of 2026-11-17.

- [ ] AC1: A passing push ends in `run.succeeded` with one `passed` check and
      a stored log per attribute; a NixOS VM test covers git-server, CI and
      cache together.
  - Status: behaviour done and unit tested
    (`pipeline_caches_skips_builds_and_logs_per_derivation`); the NixOS VM
    test does not exist yet (T11).
- [x] AC2: The same commit again, outputs cached, builds nothing.
  - Validate: `cargo test -p ci_service all_cached_passes_without_building`;
    checked by hand end to end on 2026-11-16 (rerun of the same commit).
- [x] AC3: One failing and one passing check → `failed`, the passing one is
      still built (`--keep-going`), `check.failed` names the right attribute.
  - Validate: `cargo test -p ci_service webhooks_get_exactly_the_events_they_want`
- [x] AC4: `reruns = 2` builds a failing check 3 times and records 3 tries;
      capped by `max_reruns`.
  - Validate: `cargo test -p ci_service -- failed_checks_are_rerun reruns_and_timeouts_are_capped`
- [x] AC5: No builder for a system → `skipped`; a cache miss goes to a remote
      builder of that system, else the CI host, and the check records which.
  - Validate: `cargo test -p ci_service cached_wins_then_buildable_then_skipped`;
    `nix build .#checks.x86_64-linux.ci-builder-fast`. Not tested: a real
    build over SSH.
- [ ] AC6: A user without access gets 403 on runs and logs.
  - Status: the check is in the application layer and tested with the
    scripted access adapter
    (`cargo test -p ci_service only_users_with_access_read_a_repos_runs_and_logs`);
    production uses `OpenAccess` until repohub has credentials (spec open
    question 2, T09).
- [x] AC7: A `run.failed` webhook gets exactly one POST per failed run; a
      down URL is retried then given up without blocking others.
  - Validate: `cargo test -p ci_service -- webhooks_get_exactly a_down_webhook_is_retried`
- [ ] AC8: `nixConfig` with `sandbox = false`, or IFD, cannot change settings
      or build during evaluation.
  - Status: the options are passed on every `nix-eval-jobs` call; no test
    proves the refusal yet (T11).
- [x] AC9: Application tested with in-memory adapters only; `domain/` imports
      no `tokio`, `axum` or `sqlx`.
  - Validate: `cargo test -p ci_service --lib application`;
    `grep -rn 'tokio\|axum\|sqlx' ci_service/src/domain` prints nothing.
- [x] AC10: Changing only `src/` between two pushes evaluates the config once.
  - Validate: config cached by blob id (`adapters/nix/config_source.rs`),
    `cargo test -p ci_service config`.
- [ ] AC11: The monolith runs CI, repohub and cache in one process; a UI log
      read reaches `ci_service::Service` directly and access goes through
      `RepohubAccess`.
  - Status: monolith done (step 6); `RepohubAccess` blocked (T09).
- [x] AC12: A run on `deploy.branch` with `deploy.url` delivers its
      deployment once (embedded control plane in the monolith, POST
      otherwise), retried until it succeeds.
  - Validate: `cargo test -p ci_service deploy`;
    `cargo test -p control_plane -- deploys_number_generations embedded`;
    checked by hand with the real monolith on 2026-11-16.
- [x] AC13: `cargo test -p ci_service` builds none of `repohub`, `cache`,
      `control_plane`.
  - Validate: `grep -n 'repohub\|cache\|control_plane' ci_service/Cargo.toml`
    prints no dependency.

### Full validation

- `cargo test -p ci_service` (97 pass on 2026-11-16)
- `cargo test -p control_plane` (32 pass)
- `cargo clippy -p ci_service -p control_plane --all-targets -- -D warnings`
- `cargo clippy -p procurator --no-deps -- -D warnings` (`repo_outils` has
  its own existing clippy errors, hence `--no-deps`)
- `nix build .#checks.x86_64-linux.{ci-service-fast,ci-builder-fast,deployment-fast,services-system-eval,control-plane-fast}`

### Context sync

- `ci_service/README.md` — layout, API, config, deploy, `## Status`.
- `context/specs/ci-service.md` — build order status, open questions.
- `context/context-map.md`, `context/overview.md` — module map, current work.
- `context/control-plane/node-state.md` — `Deploy` event, embedded mode.

## Task context synchronization lifecycle

- Every task carries `pending | synced | blocked`. A completed task must be
  `synced` before another task starts.
- For `blocked`, record **Blocker**, **Required action**, **Retry
  condition**.

## Constraints and non-goals

- **Constraints:** `.claude/CLAUDE.md` rules (private fields and functions,
  getters only for read fields, no `pub(crate)`/`pub(super)`, no needless
  `clone`/`to_string`, little `Arc`/`Mutex`, `impl Future` returns, no
  `type` aliases, code-free `mod.rs`, no python); clippy `all` + `pedantic`
  denied; `ci_service` depends on no other service crate; security options
  on every evaluation (no `accept-flake-config`, CI user not trusted, no
  IFD, restrict-eval).
- **Non-goals now:** editing config from a UI, a `deploy` schema beyond
  `url` and `branch`, cancelling concurrent runs.

## Task stack

- [x] T01: `Skeleton: domain, ports, SQLite run queue, run API` (status:done)
  - Done: domain types, ports, in-memory adapters, `RunStore` on SQLite,
    claim with `RETURNING`, stale-run recovery, run API, new hook payload.
  - Completed: 2026-11-15 (`2aff473`)
  - Context synchronization: synced

- [x] T02: `Evaluate and build` (status:done)
  - Done: `nix-eval-jobs` + one `nix build --keep-going`, internal-json
    parsing per derivation, per-check status and logs, buildable systems
    from `nix config show`, `max_concurrent_runs` workers, NixOS module PATH,
    `git` group, `safe.directory`.
  - Completed: 2026-11-15 (`4b8d66d`)
  - Context synchronization: synced

- [x] T03: `Repo config: procurator.nix` (status:done)
  - Done: read by blob from the checked commit, evaluated alone and
    restricted, fully validated, cached by blob id; `ci.checks`,
    `ci.reruns`, `ci.timeouts`, service caps.
  - Completed: 2026-11-16 (`30a5d02`)
  - Context synchronization: synced

- [x] T04: `Push what a run built to the cache` (status:done)
  - Done: `nix copy --no-recursive` of built outputs with netrc, owner's
    private cache unless `ci.cache` says public, push failure is a warning;
    NixOS `cache.*` options.
  - Completed: 2026-11-16 (`f8569cb`)
  - Context synchronization: synced

- [x] T05: `Events: outbox and webhooks` (status:done)
  - Done: `deliveries` outbox written with the final status, dispatcher,
    webhook adapter with backoff, URLs from `secrets_file`, `run.recovered`.
  - Completed: 2026-11-16 (`00be6ea`)
  - Context synchronization: synced

- [x] T06: `Monolith, access, live and dependency logs, retention` (status:done)
  - Done: `Access` port on every read (`OpenAccess` only), SSE follow with
    sequence numbers, dependency logs per check, log retention task, root
    `procurator` binary running CI + repohub + cache.
  - Completed: 2026-11-16 (`d931b26`)
  - Context synchronization: synced

- [x] T07: `Remote builders (NixOS)` (status:done)
  - Done: `services.procurator.ci.remoteBuilders` → `nix.buildMachines`;
    `services.procurator.ciBuilder` builder host; `ci-builder-fast` check.
  - Completed: 2026-11-16 (`8533bac`)
  - Context synchronization: synced

- [x] T08: `Deploy passing runs` (status:done)
  - Done: `deploy.url`/`deploy.branch` from the default branch;
    `procuratorDeployment` flake output built on the deploy branch,
    validated (`domain/deploy.rs`) and queued as a `Deploy` delivery;
    `Routed` notifier; `HttpDeploy` (standalone) and `ControlPlaneDeploy`
    (monolith, `control_plane::start` + `Publisher`, node numbers
    generations and skips the active intent); `libs.<system>.deployment`
    and `deployment-fast`; example in `nix/examples/python-workload`.
  - Decisions: keep `deploy.url` (no `deploy.enable`); deploy content is a
    flake output, not CI-only config.
  - Completed: 2026-11-16 (`b83d0eb`)
  - Context synchronization: synced

- [ ] T09: `Split deployment: RepohubAccess / JWT Access adapter` (status:blocked)
  - Scope: In — an `Access` adapter backed by repohub (direct call in the
    monolith, its API or a JWT when split). Out — new repohub tables beyond
    what access needs. (The HTTP deploy adapter already landed in T08.)
  - Blocker: repohub authenticates nothing and has no "who may read a repo"
    rule (spec open question 2).
  - Required action: add credentials and repo read permissions to repohub,
    then write the adapter and make AC6/AC11 real.
  - Retry condition: repohub exposes authenticate + can-read use cases.
  - Context synchronization: blocked

- [ ] T10: `Isolation: VM Builder adapter` (status:pending)
  - Scope: a `Builder` adapter that builds each run in an ephemeral VM
    instead of on the host's Nix daemon. Needs its own design first.
  - Dependencies: none hard; benefits from T13.
  - Context synchronization: pending

- [ ] T11: `End-to-end NixOS VM test` (status:pending)
  - Scope: git-server + CI + cache in one NixOS test (AC1), plus a flake
    with `sandbox = false` / IFD proving AC8.
  - Context synchronization: pending

- [ ] T12: `Hook fixes: port and token` (status:pending)
  - Scope: `nix/modules/repohub/post-receive` posts to
    `localhost:3000/api/runs` but CI listens on `8082`; enqueueing is not
    authenticated (`hook_token_hash` still to do).
  - Context synchronization: pending

- [ ] T13: `Monolith packaging` (status:pending)
  - Scope: a flake package and a NixOS module for the root `procurator`
    binary (with optional `control_plane` config).
  - Context synchronization: pending

- [ ] T14: `Deploy follow-ups` (status:pending)
  - Scope:
    - workers fetch deployed store paths from the cache (today they must
      already have them);
    - ordering: a run that finishes after a newer one (or a retried failed
      deploy) can deploy over it;
    - one repo per control plane, since each deploy replaces the whole
      desired state;
    - the worker's decoding of the CI-encoded `VmSpec` is not tested.
  - Context synchronization: pending

## Next

T09 is blocked on repohub. Unblocked choices: T12 (small), T11, T13, T14, or
the T10 design.
