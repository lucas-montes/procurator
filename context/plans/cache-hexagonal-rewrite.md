# Plan: cache-hexagonal-rewrite

## Change summary

Rewrites the `cache` crate as an embeddable library in a hexagonal layout
(`domain` / `application` / `adapters`, the convention `repohub` already uses),
with the HTTP server as one inbound adapter. One process serves several named
caches, each public (anonymous read, token write) or private (token read and
write), declared in one TOML file. Access tokens live hashed in the index and
are managed through an admin HTTP API, bootstrapped by a root token hash in
config.

Storage becomes two tool-agnostic primitives: a content-addressed blob store
(`sha256 -> bytes`, verified while streaming, deduplicated) and a SQLite index
that is the source of truth for what each cache holds. Nix is the only
front-end built here; Bazel and REAPI are meant to be added on the same
primitives later.

This replaces the current implementation. All TLS and mTLS code, the
`/nix/store` fallback, `/log`, mass-query, and the `[access]` config are
removed. The pure Nix logic (narinfo, fingerprint, signing, nix32) is carried
over.

## Acceptance criteria

- [x] AC1: On a public cache an anonymous read succeeds, an anonymous write is
      refused with 401, a read-only token's write is refused with 403, and a
      write token's upload succeeds.
  - Validate: `cargo test -p cache --test http public_cache_access_matrix`
- [x] AC2: On a private cache an anonymous read is refused with 401, a token
      with no grant for that cache is refused with 403, and a read token reads.
  - Validate: `cargo test -p cache --test http private_cache_access_matrix`
- [x] AC3: Two caches in one process are isolated: an object pushed to one is
      a 404 in the other, and each signs with its own key.
  - Validate: `cargo test -p cache --test http caches_are_isolated`
- [x] AC4: An upload whose bytes do not hash to its file name is rejected with
      400 and leaves nothing behind; concurrent uploads of one NAR stay intact.
  - Validate: `cargo test -p cache --test http -- nar_hash_mismatch_rejected concurrent_upload_is_intact`
- [x] AC5: Tokens are created, listed, and revoked through the admin API by an
      admin token only, and a revoked token stops working.
  - Validate: `cargo test -p cache --test http token_admin_api`
- [x] AC6: A real Nix client pushes to a private cache with a netrc token and
      pulls back into an empty store trusting only the cache's public key.
  - Validate: `cargo test -p cache --test nix_e2e -- --ignored`
- [x] AC7: The use cases run with no HTTP, disk, or SQLite: the application
      layer is tested against in-memory adapters, and `domain` imports no
      `tokio`, `axum`, or `sqlx`.
  - Validate: `cargo test -p cache --lib application`; `grep -rn 'tokio\|axum\|sqlx' cache/src/domain` prints nothing.
- [x] AC8: The crate contains no TLS code and never reads the host's Nix store.
  - Validate: `grep -rn 'rustls\|dump-path\|path-info' cache/src cache/Cargo.toml` prints nothing.

### Full validation

- `cargo test -p cache`
- `cargo clippy -p cache --all-targets --all-features -- -D warnings`
- `cargo fmt -p cache -- --check`
- `cargo build --workspace`
- `nix eval -f ./nix/tests/cache-fast.nix --apply "f: f {}"`

### Context sync

- `cache/README.md` — walkthrough, config reference, admin API, `## Status`.
- `context/overview.md` and `context/context-map.md` — cache description and
  module layout.
- `context/glossary.md` — named cache, visibility, grant.

## Task context synchronization lifecycle

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cache/`, `nix/modules/cache.nix`, `nix/tests/cache-fast.nix`,
  `example/flake.nix` and `example/README.md` where they name the cache URL,
  `Cargo.lock`, and the context files listed above.
- **Out of scope:** S3 or other remote blob backends, the Bazel HTTP front-end,
  REAPI gRPC, TLS of any kind (terminate it in front of the service), request
  idle timeouts, token expiry, and wiring other crates to embed the cache.
- **Constraints:** `.claude/CLAUDE.md` rules (private fields, minimal
  visibility, no `pub(crate)`, avoid needless allocation and `Arc`/`Mutex`,
  `impl Future` returns); static dispatch, no `dyn`; concrete error enums, no
  `anyhow`; config from one TOML file; workspace `clippy::pedantic = deny`;
  the workspace `sqlx` dependency for the index.
- **Non-goal:** no generic artifact-protocol abstraction beyond the blob store
  and index, and no data migration from the old flat `storage_dir` layout.

## Assumptions

- No deployment holds data worth migrating in the old storage layout.
- Caches are addressed by URL prefix: `http://host/<cache>/`.
- A signing key is required for every Nix cache; the server always adds its
  signature to an uploaded narinfo.
- An admin token has full access to every cache as well as the token API.
- A cache at its quota rejects uploads with 507; GC frees space by retention
  age and by evicting least-recently-read paths above a high-water mark.

## Task stack

- [x] T01: `Lay out the crate as domain, application, and adapters` (status:done)
  - Task ID: T01
  - Scope: In — `lib.rs` + `main.rs`, the three layers, `http`/`sqlite`
    features, removal of the old modules and TLS dependencies. Out — behavior.
  - Dependencies: none
  - Done when: the crate builds as a library plus a binary with the old
    modules gone.
  - Verify: `cargo build -p cache --all-features`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T02: `Model caches, digests, and access in the domain` (status:done)
  - Task ID: T02
  - Scope: In — `CacheName`, `Visibility`, `Digest` (hex, nix32), `Principal`,
    grants, and the pure authorize decision. Out — IO.
  - Dependencies: T01
  - Done when: the access matrix for public and private caches is unit tested.
  - Verify: `cargo test -p cache --lib domain`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T03: `Port the Nix narinfo and signing logic into the domain` (status:done)
  - Task ID: T03
  - Scope: In — narinfo parse/render, NAR file names, fingerprint, ed25519
    signing, signing-key parsing. Out — store introspection (deleted).
  - Dependencies: T01
  - Done when: the carried-over tests pass and signatures verify with the
    matching public key.
  - Verify: `cargo test -p cache --lib domain::nix`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T04: `Define ports and in-memory adapters` (status:done)
  - Task ID: T04
  - Scope: In — `BlobStore`, `Index`, `Clock`, `Entropy` ports; in-memory blob
    store and index. Out — disk and SQLite.
  - Dependencies: T02
  - Done when: the in-memory adapters satisfy the ports.
  - Verify: `cargo test -p cache --lib`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T05: `Implement the Nix cache use cases` (status:done)
  - Task ID: T05
  - Scope: In — cache info, narinfo read/write with signing, NAR read/write
    with hash verification and quota, all authorizing a `Principal`. Out — HTTP.
  - Dependencies: T03, T04
  - Done when: push/pull, hash mismatch, quota, and the access matrix pass
    against in-memory adapters.
  - Verify: `cargo test -p cache --lib application`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T06: `Implement authentication and token administration` (status:done)
  - Task ID: T06
  - Scope: In — credential to `Principal`, root token from config, create/list
    /revoke use cases. Out — token expiry.
  - Dependencies: T04
  - Done when: only admins manage tokens and a revoked token is rejected.
  - Verify: `cargo test -p cache --lib application`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T07: `Implement retention, eviction, and orphan sweep` (status:done)
  - Task ID: T07
  - Scope: In — one GC pass: age retention, high-water LRU eviction, unlinked
    NARs, unreferenced blobs, guarded against concurrent uploads. Out —
    closure-aware GC.
  - Dependencies: T05
  - Done when: a GC pass frees expired entries and their blobs and never
    removes a blob a cache still references.
  - Verify: `cargo test -p cache --lib application`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T08: `Add the local-disk blob store` (status:done)
  - Task ID: T08
  - Scope: In — streaming stage with SHA-256, unique staging files cleaned on
    drop and at startup, atomic commit, list/delete. Out — S3.
  - Dependencies: T04
  - Done when: staged bytes round-trip and an oversized stream is refused
    without leaving a file.
  - Verify: `cargo test -p cache --lib adapters`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T09: `Add the SQLite index` (status:done)
  - Task ID: T09
  - Scope: In — schema creation and every `Index` operation on `sqlx`. Out —
    Postgres.
  - Dependencies: T04
  - Done when: the same index contract tests pass for memory and SQLite.
  - Verify: `cargo test -p cache --lib adapters`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T10: `Expose the Nix and admin HTTP routes` (status:done)
  - Task ID: T10
  - Scope: In — `/<cache>/` Nix routes, `/_api/v1/tokens`, health, readiness,
    metrics, credential extraction, error-to-status mapping, integration tests.
    Out — TLS.
  - Dependencies: T05, T06, T08, T09
  - Done when: AC1–AC5 pass over HTTP.
  - Verify: `cargo test -p cache --test http`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T11: `Wire config, GC loop, and graceful shutdown in the binary` (status:done)
  - Task ID: T11
  - Scope: In — new TOML schema with validation, composition root, periodic
    GC, shutdown on SIGINT/SIGTERM, `example.toml`. Out — hot reload.
  - Dependencies: T07, T10
  - Done when: `cargo run -p cache -- --config ./cache/example.toml` serves a
    public and a private cache.
  - Verify: `curl -fsS http://127.0.0.1:8081/public/nix-cache-info`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T12: `Add a real Nix client end-to-end test` (status:done)
  - Task ID: T12
  - Scope: In — an `#[ignore]`d test pushing and pulling with `nix copy` and a
    netrc token. Out — NixOS VM tests.
  - Dependencies: T11
  - Done when: the test passes with `nix` on PATH and is skipped by default.
  - Verify: `cargo test -p cache --test nix_e2e -- --ignored`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T13: `Update the NixOS module, its test, and the example flake` (status:done)
  - Task ID: T13
  - Scope: In — `nix/modules/cache.nix`, `nix/tests/cache-fast.nix`, the
    example's substituter URL. Out — secrets tooling.
  - Dependencies: T11
  - Done when: the module emits a config the binary accepts and the fast test
    passes.
  - Verify: `nix eval -f ./nix/tests/cache-fast.nix --apply "f: f {}"`
  - Completed: 2026-11-12
  - Context synchronization: synced

- [x] T14: `Rewrite the cache README and durable context` (status:done)
  - Task ID: T14
  - Scope: In — `cache/README.md`, the cache entries in `context/overview.md`,
    `context/context-map.md`, `context/glossary.md`. Out — other crates' docs.
  - Dependencies: T12, T13
  - Done when: the README walkthrough runs as written and no doc mentions the
    removed options.
  - Verify: `grep -rn 'reads_require_auth\|writer_allowlist\|loopback_bypass\|mTLS' cache/README.md` prints nothing.
  - Completed: 2026-11-12
  - Context synchronization: synced

## Open questions

- Is building REAPI for Buck2 worth it versus running an existing server
  behind the same tokens? Out of scope here; it only affects how much more the
  blob store port must anticipate.

## Validation Report

Run on 2026-11-12 after T14. All tasks were implemented in one session, so
per-task evidence is recorded here rather than under each task.

| Check | Result |
|---|---|
| `cargo test -p cache` | pass: 40 unit tests, 7 HTTP tests |
| `cargo test -p cache --no-default-features` | pass: 36 tests (use cases and memory/disk adapters without `http` or `sqlite`) |
| `cargo test -p cache --test nix_e2e -- --ignored` | pass: real `nix copy` push and pull through a private cache; wrong key and anonymous pull both refused |
| `cargo clippy -p cache --all-targets --all-features -- -D warnings` | clean |
| `cargo fmt -p cache -- --check` | clean |
| `nix eval -f ./nix/tests/cache-fast.nix --apply "f: f {}"` | `true` |
| Binary started from the NixOS module's generated TOML | serves both caches |
| AC7 / AC8 greps | no matches |
| `cargo build --workspace` | **fails in `worker`**, not in `cache`: `worker/src/proxy/core.rs:185` moves `state.runtime_settings` out of an `Arc` (E0507). The file is unchanged from commit `26267fd`; outside this plan's scope. |

Acceptance criteria AC1–AC8 are met. Notes on how the result differs from the
plan as written:

- `#![allow(clippy::manual_async_fn)]` is set crate-wide in `cache/src/lib.rs`.
  The project rule (return `impl Future`) and that clippy lint contradict each
  other; the rule won.
- The `--raw` flag in the fast-test command documented in
  `nix/tests/cache-fast.nix` cannot print a boolean; run it without `--raw`.
- Request idle timeouts and token expiry were left out, as scoped.

Files: `cache/` (rewritten: `Cargo.toml`, `README.md`, `example.toml`,
`src/**`, `tests/**`), `nix/modules/cache.nix`, `nix/tests/cache-fast.nix`,
`nix/modules/SERVICE_MODULES.md`, `example/flake.nix`, `example/README.md`,
`context/context-map.md`, `context/glossary.md`, `Cargo.lock`.
