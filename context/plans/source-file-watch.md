# Plan: Restore per-service source file watching

Plan name: `source-file-watch`
Path: `context/plans/source-file-watch.md`

## Change Summary

The current file watcher detects file changes and re-parses the flake config, but only
restarts services whose `cmd` field changed in the config. Changes to source files
(e.g., editing `client.py` inside a service's `src` directory) do not trigger restarts
because the `cmd` field in `flake.nix` hasn't changed.

This plan restores per-service source directory watching — when a file changes inside a
service's `src` directory, that service is restarted with its existing `cmd`, without
re-parsing the flake config.

## Success Criteria

1. Editing a source file inside a service's `src` directory causes that service to be
   restarted (with a log message like `[name] restarted by source change`).
2. Editing `flake.nix` still triggers a full config re-parse + diff + restart of
   affected services (existing behavior preserved).
3. Services without a `src` field are not watched for source changes.
4. Multiple services sharing the same `src` directory are all restarted when a file
   in that directory changes.
5. `cargo build -p cli` passes, and `cargo test -p cli` passes.

## Constraints and Non-goals

- **In scope:** Adding `WatchEvent` enum, per-service source dir watching, source-change
  restart handling, wiring in cli.rs.
- **Out of scope:** Watching files imported by `flake.nix` (`import` statements).
- **Out of scope:** Re-running oneShot services on source change (they ran during initial
  start and stay stopped).
- **Out of scope:** Config file watching for per-service files other than `src` directories.
- **No new crate dependencies** — uses existing `notify` and `tokio`.

## Architecture

```
Watcher (watch.rs)                          Supervisor (service.rs)
    │                                              │
    │  WatchEvent::SourceChanged(["client"])        │  restart named services
    │═══════════════════════════════════════►        │  with existing cmd (no diff)
    │                                              │
    │  WatchEvent::ConfigChanged(new_manifest)      │  diff old↔new, restart changed
    │═══════════════════════════════════════►        │  (existing behavior)
    │                                              │
```

## Tasks

- [x] T01: Add `WatchEvent` enum and update channel types (status:done)
  - **Completed:** 2026-06-17
  - **Files changed:** `watch.rs` (added `WatchEvent` enum, changed channel type), `service.rs` (imported `WatchEvent`, changed `Supervisor` channel type, updated `run()` match), `cli.rs` (changed channel creation to `WatchEvent`)
  - **Evidence:** `cargo build -p cli` — zero errors; `cargo test -p cli` — 8/8 passed; `cargo fmt --all --check` — clean
  - **Notes:** The `SourceChanged` arm has a `tracing::warn` placeholder — filled in by T02. Borrow checker required a single `recv()` + inner `match` instead of two `select!` branches.

  **Task ID:** T01
  **Goal:** Define a `WatchEvent` enum that replaces the bare `ServiceManifest` channel type,
    allowing the watcher to distinguish config changes from source changes.

  **Boundaries (in/out of scope):**
  - In:
    - Define `pub enum WatchEvent` in `watch.rs`:
      ```rust
      pub enum WatchEvent {
          ConfigChanged(ServiceManifest),
          SourceChanged(Vec<String>),
      }
      ```
    - Change `mpsc::Sender<ServiceManifest>` → `mpsc::Sender<WatchEvent>` in `Watcher`
    - Change `mpsc::Receiver<ServiceManifest>` → `mpsc::Receiver<WatchEvent>` in `Supervisor`
    - Update `Watcher::new`, `Supervisor::new`, and channel creation in `cli.rs`
  - Out: Any logic changes to how events are handled (T02, T03).

  **Done when:** `WatchEvent` defined, channel types updated, `cargo build -p cli` compiles.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"; echo "exit=$?"
  ```

- [x] T02: Handle `WatchEvent::SourceChanged` in Supervisor (status:done)
  - **Completed:** 2026-06-17
  - **Files changed:** `service.rs` (replaced `tracing::warn` placeholder with restart logic in `SourceChanged` arm)
  - **Evidence:** `cargo build -p cli` — zero errors; `cargo test -p cli` — 8/8 passed; `cargo fmt --all --check` — clean
  - **Notes:** Restart pattern matches `apply_manifest`: remove old handle, clone `Service<Parsed>` from manifest, start, insert. OneShot filtering is the watcher's responsibility (T03).

  **Task ID:** T02
  **Goal:** Update `Supervisor::run()` to handle `WatchEvent::SourceChanged(names)` by
    restarting each named service — stop the old handle, start a new one from the current
    manifest's service config.

  **Boundaries (in/out of scope):**
  - In:
    - Change `run()` to match on `WatchEvent`:
      - `ConfigChanged(manifest)` → existing `apply_manifest` logic
      - `SourceChanged(names)` → for each name: log restart, look up the service in
        `self.manifest`, stop old handle from `running`, start new one, insert into `running`
    - Restart uses the same pattern as `apply_manifest`: remove the old `Running` handle,
      clone the `Service<Parsed>` from `self.manifest`, call `.start(logs_tx)`, insert result.
    - Log `"[name] restarted by source change"` for each restarted service.
  - Out: No changes to `apply_manifest` or the config-changed path.

  **Done when:** Supervisor restarts services on `SourceChanged`; config-changed path is
    unchanged. `cargo build -p cli` passes.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"; echo "exit=$?"
  ```

- [x] T03: Add per-service source directory map and classification in Watcher (status:done)
  - **Completed:** 2026-06-17
  - **Files changed:** `watch.rs` (added `src_dirs` field + `match_source_dirs` function + event classification in `watcher_loop`), `cli.rs` (passes empty `HashMap::new()` stub)
  - **Evidence:** `cargo build -p cli` — zero errors; `cargo test -p cli` — 8/8 passed; `cargo fmt --all --check` — clean
  - **Notes:** Classification is mutually exclusive per event batch: paths inside src_dirs → `SourceChanged`, paths outside → `ConfigChanged` (re-parse flake). Both can fire in the same batch if mixed changes occur. Empty map stub in cli.rs replaced by T04.

  **Task ID:** T03
  **Goal:** The watcher receives a map of source directories to service names, watches
    those directories, and classifies events as source changes or config changes.

  **Boundaries (in/out of scope):**
  - In:
    - Add field to `Watcher`:
      ```rust
      /// Map from source directory → service names that share it.
      src_dirs: HashMap<PathBuf, Vec<String>>,
      ```
    - Update `Watcher::new()` to accept the map.
    - In `watcher_loop`, for each incoming file event:
      - Check if the changed path is inside any watched `src_dirs` key (prefix match)
      - If yes → collect affected service names → send `WatchEvent::SourceChanged(names)`
      - If no → re-parse flake → send `WatchEvent::ConfigChanged(manifest)` (existing logic)
    - Keep watching the repo root recursively (for flake changes).
    - Services without a `src` field are not in the map and thus not watched for source changes.
  - Out: No changes to debounce logic or notify setup beyond adding the extra watches.

  **Done when:** Watcher sends `SourceChanged` for file changes inside `src` dirs and
    `ConfigChanged` for file changes outside `src` dirs. `cargo build -p cli` passes.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"; echo "exit=$?"
  ```

- [x] T04: Wire source directory map in cli.rs (status:done)
  - **Completed:** 2026-06-17
  - **Files changed:** `cli.rs` (canonicalized `self.path` → `repo_path` so src_dirs keys are absolute, matching notify's absolute event paths)
  - **Evidence:** `cargo build -p cli` — zero errors; `cargo test -p cli` — 8/8 passed; `cargo fmt --all --check` — clean
  - **Notes:** The core fix: `repo_path = self.path.canonicalize()` at the start of `execute()`. The `src_dirs` map was already being built correctly (the plan's sample code was already there); the bug was that relative `self.path` → relative keys → notify's absolute paths never matched. Canonicalization resolves this. Smoke test confirmed: `touch mock/services/client.py` → `restarted by source change name=client`.

  **Task ID:** T04
  **Goal:** Build the `src_dirs` map from the initial service graph and pass it to
    `Watcher::new`.

  **Boundaries (in/out of scope):**
  - In:
    - After parsing the initial config and building the graph, build a map:
      ```rust
      let src_dirs: HashMap<PathBuf, Vec<String>> = graph.services().iter()
          .filter_map(|(name, svc)| svc.src().map(|s| (name.clone(), self.path.join(s))))
          .fold(HashMap::new(), |mut acc, (name, path)| {
              acc.entry(path).or_default().push(name);
              acc
          });
      ```
    - Pass `src_dirs` to `Watcher::new(watcher_tx, self.path, src_dirs)`
    - Update channel creation: `mpsc::channel::<WatchEvent>(16)`
  - Out: No changes to the log writer or supervisor setup.

  **Done when:** `cargo build -p cli` passes. Smoke test: editing a file in a service's
    `src` dir produces a restart log.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"; echo "exit=$?"
  cargo run -p cli --bin pcr -- stack --path ./mock start
  # In another terminal: touch mock/services/client.py
  # Should see: "[client] restarted by source change"
  ```

- [ ] T05: Validation and cleanup (status:todo)

  **Task ID:** T05
  **Goal:** Full build, tests, formatting, manual smoke test, sync context.

  **Boundaries (in/out of scope):**
  - In: Build, unit tests, format check, manual smoke test, context sync.
  - Out: No new feature work.

  **Done when:**
  - `cargo build -p cli` — zero errors
  - `cargo test -p cli` — all tests pass
  - `cargo fmt --all --check` — clean
  - Smoke test confirms source file change restarts service
  - Smoke test confirms flake change still triggers config re-parse
  - Working tree clean

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"
  cargo test -p cli 2>&1 | grep -E "test result|FAILED"
  cargo fmt --all --check
  ```

## Open Questions

None — requirements are settled.
