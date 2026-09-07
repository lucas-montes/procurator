# Plan: Use RUST_LOG env var for log level

Plan name: `rust-log-env-var`
Path: `context/plans/rust-log-env-var.md`

## Change Summary

The app currently hard-codes `tracing_subscriber::fmt().with_env_filter("info")` in both
the CLI and worker binaries. This ignores the standard `RUST_LOG` environment variable
used across the Rust ecosystem.

Switch to `EnvFilter::try_from_default_env()` with fallback to `"info"`, so that
`RUST_LOG=debug pcr` works without requiring code changes.

## Success Criteria

1. When `RUST_LOG` is **not** set, log level defaults to `info` (existing behavior preserved).
2. When `RUST_LOG=debug` is set, debug-level and above messages are printed.
3. When `RUST_LOG=error` is set, only error-level messages are printed.
4. Both binaries (`pcr` and `pcr_worker_test`) respect the env var.
5. `cargo build -p cli` passes with zero errors.

## Constraints and Non-goals

- **In scope:** Both `cli/pcr/main.rs` and `cli/worker/main.rs`.
- **Out of scope:** Any other tracing-subscriber configuration changes (format, output target, etc.).
- **Out of scope:** The stack-level `LogConfig` / `LogWriter` — that subsystem is independent of app-level tracing.
- **No new dependencies** — `env-filter` feature is already enabled at workspace level.

## Tasks

- [x] T01: Switch to `EnvFilter::try_from_default_env()` with `"info"` fallback (status:done)
  - **Completed:** 2026-06-17
  - **Files changed:** `cli/pcr/main.rs`, `cli/worker/main.rs` (both replaced `with_env_filter("info")` with `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`)
  - **Evidence:** `cargo build -p cli` — zero errors; `cargo test -p cli` — 8/8 passed; `cargo fmt --all --check` — clean
  - **Notes:** No new imports needed (fully-qualified `EnvFilter` path). Smoke tests confirmed: default → info logs visible; `RUST_LOG=error` → 0 info lines.

  **Task ID:** T01
  **Goal:** Replace the hard-coded `with_env_filter("info")` call in both `main.rs` files
    with an `EnvFilter` that reads `RUST_LOG` and falls back to `info`.

  **Boundaries (in/out of scope):**
  - In:
    - `cli/pcr/main.rs` line 55: replace `with_env_filter("info")`
      ```rust
      // Before:
      tracing_subscriber::fmt().with_env_filter("info").init();
      // After:
      tracing_subscriber::fmt()
          .with_env_filter(
              tracing_subscriber::EnvFilter::try_from_default_env()
                  .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
          )
          .init();
      ```
    - `cli/worker/main.rs` line 125: same change.
  - Out: No other logger changes. No new imports needed (the `EnvFilter` path is fully qualified).

  **Done when:** Both binaries use `RUST_LOG` env var; `cargo build -p cli` compiles;
    `RUST_LOG=error cargo run --bin pcr -- stack --path ./mock start` shows only errors.

  **Verification:**
  ```bash
  cargo build -p cli 2>&1 | grep "^error"
  RUST_LOG=error cargo run --bin pcr -- stack --path ./mock start &
  sleep 3; kill $! 2>/dev/null; wait $! 2>/dev/null
  # Should see no info-level logs, only errors (if any)
  ```

- [x] T02: Validation and context sync (status:done)

  **Task ID:** T02
  **Goal:** Final build pass, smoke test both RUST_LOG unset and set, update context.

  **Boundaries (in/out of scope):**
  - In: Build, manual smoke test with and without RUST_LOG, context sync.
  - Out: No new feature work.

  **Done when:**
  - `cargo build -p cli` — zero errors
  - `cargo fmt --all --check` — clean
  - Default behavior (no RUST_LOG) confirmed: info-level logs appear
  - `RUST_LOG=error` confirmed: info-level logs suppressed
  - Working tree clean

  **Verification:**
  ```bash
  cargo build -p cli
  cargo fmt --all --check
  ```
