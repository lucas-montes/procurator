# Plan: worker-opencode-t06-unblock

## 1) Change summary
Unblock and close `T06` from `worker-opencode-public-proxy` by addressing final validation blockers:
1) TAP tests that fail in non-privileged environments (`Operation not permitted`), and
2) strict lint failures under `cargo clippy -p worker --all-targets -- -D warnings`.

Per user direction, TAP tests will be decorated to skip normal runs, and the worker crate will be remediated to pass strict clippy warnings across all worker modules.

## 2) Success criteria
- TAP ioctl-dependent tests in `worker` no longer fail default local/CI test runs due to missing capabilities.
- TAP tests remain runnable intentionally (explicit opt-in) for privileged environments.
- `cargo clippy -p worker --all-targets -- -D warnings` passes without reducing lint strictness for the worker crate.
- `cargo test -p worker` passes in non-privileged environments.
- `cargo fmt --all -- --check` passes.
- Original blocked task `T06` in `worker-opencode-public-proxy` is updated from blocked to done with full evidence.

## 3) Constraints and non-goals
### Constraints
- Scope is limited to unblocking validation for the worker crate and closing original `T06`.
- Keep proxy runtime behavior unchanged unless required to satisfy lint correctness.
- Do not relax global clippy gate semantics; fix warnings instead of disabling lint policy.
- TAP test skip behavior must be explicit in test metadata/comments so privileged execution remains discoverable.

### Non-goals
- No new proxy features.
- No control-plane protocol changes.
- No redesign of TAP subsystem behavior beyond test execution strategy.

## 4) Task stack (`T01..T04`)
- [ ] T01: `Decorate TAP privileged tests to skip by default` (status:todo)
  - Task ID: T01
  - Goal: Mark TAP ioctl-dependent tests with explicit skip/ignore decorators so default `cargo test -p worker` does not attempt privileged TAP creation.
  - Boundaries (in/out of scope): In - TAP test metadata and comments/instructions for explicit privileged execution. Out - TAP runtime production code behavior changes.
  - Done when: Known TAP permission-failing tests are skipped in default runs and clearly labeled as privileged-only.
  - Verification notes (commands or checks): `cargo test -p worker ch::tap::tests`; verify skipped status for decorated tests in non-privileged env.

- [ ] T02: `Remediate strict clippy warnings across worker crate` (status:todo)
  - Task ID: T02
  - Goal: Fix all worker crate clippy issues required for `-D warnings` across all targets.
  - Boundaries (in/out of scope): In - code/style/documentation-driven refactors in `worker` needed for strict clippy pass. Out - unrelated crates/modules outside `worker`.
  - Done when: `cargo clippy -p worker --all-targets -- -D warnings` succeeds with no allow-by-default policy weakening.
  - Verification notes (commands or checks): run exact clippy command; capture zero-warning pass.

- [ ] T03: `Re-run full T06 validation gates and close prior blocked task` (status:todo)
  - Task ID: T03
  - Goal: Execute the full validation command set and update original plan task `T06` to done with final evidence.
  - Boundaries (in/out of scope): In - validation command execution + evidence updates in both plans. Out - new feature implementation.
  - Done when: all required gates pass (`test`, `clippy`, `fmt`) and `worker-opencode-public-proxy` T06 is marked done with evidence.
  - Verification notes (commands or checks): `cargo test -p worker`; `cargo clippy -p worker --all-targets -- -D warnings`; `cargo fmt --all -- --check`.

- [ ] T04: `Validation cleanup and context sync` (status:todo)
  - Task ID: T04
  - Goal: Ensure context reflects final unblocked validation state and remove any temporary unblock notes that are no longer current-state accurate.
  - Boundaries (in/out of scope): In - final `context/` reconciliation for worker validation posture and plan closure state. Out - further code changes unless needed for documentation correctness.
  - Done when: context files accurately represent that proxy plan validation is fully closed and privileged TAP test expectations are documented.
  - Verification notes (commands or checks): context sync pass over shared files + `context/worker/public-proxy.md`; link verification via `context/context-map.md`.

## 5) Open questions (if any)
- None blocking. User selected explicit skip-decorator strategy for TAP privileged tests, full worker clippy remediation scope, and completion target to close original blocked `T06`.
