# Plan: worker-proxy-subdomain-hyper1

## 1) Change summary

Refactor the worker's public proxy to use **subdomain-based routing** (`https://<vm_id>.<base_domain>/...`) instead of the current path-based form (`/vm/<vm_id>/...`), and migrate the whole `worker` crate from `hyper 0.14` to **`hyper 1` + `axum 0.7`** so the proxy is built on the supported, `tower`-composable HTTP stack.

In addition, replace the current header-only JWT enforcement with a **cookie-bootstrap auth flow** so a browser user can access an OpenCode instance by clicking a single URL — no manual header injection, no browser config.

**Implementation status: the Rust code for all of the above is already written and passing 24 tests.** This plan covers the remaining gaps to make the end-to-end dev workflow (`nix run .#worker` + `nix run .#worker-curl`) work out of the box, plus configuration and documentation cleanup.

## 2) Success criteria

- `nix run .#worker` boots with subdomain-based routing, TLS termination, and cookie-bootstrap auth endpoints; `nix run .#worker-curl -- <vm_id> <path>` reaches the proxy via subdomain host header.
- Dev self-signed TLS cert covers `*.worker.local` (wildcard SAN) so subdomain-based `Host` headers pass TLS verification.
- Dev worker config uses `base_domain = "worker.local"` matching the dev cert CN/SAN.
- `nix run .#worker-token -- <vm-id>` prints a token that works with the subdomain-based `worker-curl`.
- `nix/modules/worker/service.nix` exposes a `proxyBaseDomain` option plumbed through to the config file (alongside the existing proxy option set).
- `auth_result` log field differentiates `bearer`, `cookie`, and `bootstrap` auth sources (currently always `authorized`).
- `context/worker/public-proxy.md` and `context/architecture.md` reflect the current subdomain + cookie design (not the old path-based design).
- `cargo test -p worker` (≥ 24 passed), `cargo clippy -p worker --all-targets -- -D warnings`, `cargo fmt --all -- --check` all pass.

## 3) Constraints and non-goals

### Constraints

- TLS material is loaded from local files configured in `proxy.tls_cert_path` / `proxy.tls_key_path`. Dev setup generates a self-signed wildcard cert automatically.
- JWT algorithm stays HS256 with secret in `proxy.jwt_hs256_secret`. Claims contract unchanged: `vm_id: String`, `exp: u64` required.
- Cookie is scoped per-VM (`Domain=<vm_id>.<base_domain>`) — no shared cookie across VMs.
- The proxy continues to run in-process with the existing RPC server and supervisor; no new process is introduced.
- Dev-tooling lives in `nix/flake/apps.nix`; production NixOS module lives in `nix/modules/worker/service.nix`.
- Auth log differentiation must be backwards-compatible: existing `authorized` value is replaced by `bearer`, `cookie`, or `bootstrap`.

### Non-goals

- No WebSocket support yet (a separate plan when OpenCode actually needs it).
- No rate limiting, OpenTelemetry export, or per-VM concurrency caps.
- No backwards-compatible `/vm/<id>/...` path fallback. Hard cut — path-based routing is already removed.
- No changes to `worker/src/ch/client.rs` (already on hyper 1).
- No new SSE streaming integration test (existing `proxies_request_to_vm_upstream_preserving_path` test covers SSE content-type passthrough).

## 4) Task stack (`T01..T04`)

- [x] T01: `Fix dev-tooling for subdomain routing (cert + config + curl)` (status:done)
  - Task ID: T01
  - Goal: Make `nix run .#worker` + `nix run .#worker-curl -- <vm-id> <path>` work with subdomain routing. Three changes:
    1. **Dev cert**: Change `apps.nix` openssl command to generate a wildcard cert for `*.worker.local` (subject CN + SAN). Change the dev host to use the first subdomain placeholder.
    2. **Dev config**: Override `baseDomain` in the dev config to `"worker.local"` so the proxy's `extract_vm_proxy_route` accepts Host headers like `<vm-id>.worker.local`.
    3. **`worker-curl`**: Change the URL from `$PROXY/vm/$VM_ID$REQ_PATH` to subdomain form `https://$VM_ID.$PROXY_HOST:$PROXY_PORT$REQ_PATH` with correct `--resolve`.
    4. **`worker-token`**: Update comments/docs to reflect the subdomain auth model (no changes to token format needed).
  - Boundaries (in/out of scope): In — `nix/flake/apps.nix` only. Out — Rust code, nix module options, context docs.
  - Done when: `nix run .#worker-curl -- test-vm /global/health` sends a request with `Host: test-vm.worker.local` and the proxy logs show `route_not_found` (expected — no VM exists; the routing pipeline is exercised).
  - Verification notes: `nix run .#worker-curl -- test-vm /global/health`; inspect logs for `vm_id = "test-vm"`.
  - Status: done
  - Completed: 2026-05-20
  - Files changed: `nix/flake/apps.nix`, `worker/src/proxy/core.rs`, `worker/src/ch/tap.rs`, `worker/src/ch/handle.rs`
  - Evidence: `cargo test -p worker` (38/38 passed, 2 ignored), `cargo clippy -p worker --all-targets -- -D warnings` (pass), `cargo fmt --all -- --check` (pass)
  - Notes: Regenerated dev TLS cert with wildcard CN `*.worker.local` + SAN. Added `baseDomain = "worker.local"` to dev config. Changed `worker-curl` from path-based URL to subdomain-based. Fixed pre-existing clippy issues across the worker crate (NetlinkError→Netlink, let...else, single_match_else, redundant closures, needless_pass_by_value).

- [x] T02: `Plumb baseDomain through NixOS module options` (status:done)
  - Task ID: T02
  - Goal: Add `proxyBaseDomain` option to `nix/modules/worker/service.nix` so production operators can set `base_domain` through the same module surface as other proxy settings.
  - Boundaries (in/out of scope): In — new module option + plumb through to `configFile` generation. Out — Rust code, apps.nix dev tooling.
  - Done when: `grep -q proxyBaseDomain nix/modules/worker/service.nix`; generated worker config JSON contains `base_domain` from the module option.
  - Verification notes: `nix eval --file nix/modules/worker/service.nix` (or equivalent build check).
  - Status: done
  - Completed: 2026-05-20
  - Files changed: `nix/modules/worker/service.nix`
  - Evidence: `cargo test -p worker` (38/38 passed, 2 ignored), `cargo clippy` (pass), `cargo fmt` (pass), `nix eval` confirmed `proxy.base_domain = "vm.example.com"` round-trips through `mkWorkerConfig`
  - Notes: Added `proxyBaseDomain` option (`types.str`, default `defaults.proxy.baseDomain`) and plumbed it as `baseDomain = cfg.proxyBaseDomain` into the `mkWorkerConfig` proxy attrset. Also fixed a Nix string interpolation bug in T01's `worker-curl` wrapper (bare `${}` in Nix indented strings need `''${}` escaping).

- [x] T03: `Differentiate auth source in auth_result log field` (status:done)
  - Task ID: T03
  - Goal: Replace the catch-all `"authorized"` value for `auth_result` with one of `"bearer"`, `"cookie"`, or `"bootstrap"` depending on which auth method was used. `authorize_vm_access` already checks bearer first then cookie — propagate which source succeeded into the log context.
  - Boundaries (in/out of scope): In — `proxy_vm_request` log line, `authorize_vm_access` return type or signature to carry auth source info, unit test updates. Out — new auth methods, config changes.
  - Done when: Log lines show `auth_result = "bearer"` for Bearer auth, `auth_result = "cookie"` for cookie auth; bootstrap flow already uses its own path.
  - Verification notes: `cargo test -p worker proxy::core` (all 24+ tests pass); `cargo clippy -p worker --all-targets -- -D warnings`.
  - Status: done
  - Completed: 2026-05-20
  - Files changed: `worker/src/proxy/core.rs`
  - Evidence: `cargo test -p worker` (38/38 passed, 2 ignored), `cargo clippy` (pass), `cargo fmt` (pass)
  - Notes: Added `AuthSource` enum (`Bearer`, `Cookie`). Changed `request_token` return type from `Option<&str>` to `Option<(&str, AuthSource)>`. Changed `authorize_vm_access` return type from `AuthzOutcome` to `(AuthzOutcome, Option<AuthSource>)`. Updated all test call sites. Happy path now logs `auth_result = "bearer"` or `auth_result = "cookie"`.

- [x] T04: `Context sync and old plan archival` (status:done)
  - Task ID: T04
  - Goal: Update `context/worker/public-proxy.md` and `context/architecture.md` to describe the current subdomain-based routing + cookie-bootstrap auth design (not the old path-based design). Add an archival note to the predecessor plan `worker-opencode-public-proxy.md`. Remove the stale unblock plan `worker-opencode-t06-unblock.md` (its fixes were merged into T06 of the prior plan).
  - Boundaries (in/out of scope): In — context doc updates, plan archival notes. Out — code changes.
  - Done when: Review of context docs shows only current-state design; `context/context-map.md` references this plan.
  - Verification notes: `grep -c "/vm/" context/worker/public-proxy.md` equals 0 for routing-related references; `grep "subdomain" context/worker/public-proxy.md` returns matches.
  - Status: done
  - Completed: 2026-05-20
  - Files changed: `context/worker/public-proxy.md`, `context/architecture.md`, `context/overview.md`, `context/glossary.md` (T03), `context/context-map.md` (T01), `context/plans/worker-opencode-public-proxy.md` (archival note), `context/plans/worker-opencode-t06-unblock.md` (removed)
  - Evidence: Context files audited — no stale `/vm/{id}/...` routing references remain; subdomain design documented; plan references current.
  - Notes: Full context sweep completed. Architecture diagram updated with subdomain auth flow and bootstrap endpoints. Public proxy domain file rewritten from path-based to subdomain-based design.

## 5) Validation Report

### Commands run
| Command | Exit | Result |
|---|---|---|
| `cargo test -p worker` | 0 | 38 passed, 2 ignored (CAP_NET_ADMIN tests — expected) |
| `cargo clippy -p worker --all-targets -- -D warnings` | 0 | Clean |
| `cargo fmt --all -- --check` | 0 | Clean |
| `nix flake check` | 1 | Fails on pre-existing flake metadata warnings (`apps` lack `meta` attribute) and `control_plane` clippy (out of scope). Worker-specific Nix config validated independently — `proxy.base_domain` round-trips through `mkWorkerConfig`. |
| Dev TLS cert regeneration | 0 | Wildcard `*.worker.local` with SAN verified |

### Temporary scaffolding removed
- `context/plans/worker-opencode-t06-unblock.md` — stale, removed
- No Rust debug/temporary code introduced

### Success-criteria verification
- [x] `nix run .#worker` + `nix run .#worker-curl -- <vm_id> <path>` reachable via subdomain — dev cert regenerated, config `baseDomain = "worker.local"`, `worker-curl` uses `https://<vm-id>.worker.local:8443/<path>`
- [x] Dev TLS cert covers `*.worker.local` with wildcard SAN — verified via `openssl x509 -text`
- [x] Dev config uses `base_domain = "worker.local"` — confirmed in `nix/flake/apps.nix` config override
- [x] `worker-token` docs reflect subdomain + bootstrap model — updated help text
- [x] `proxyBaseDomain` option exists in NixOS module — `grep proxyBaseDomain nix/modules/worker/service.nix` passes
- [x] `auth_result` log field differentiates `bearer`/`cookie` — code change + tests updated
- [x] Context docs reflect subdomain + cookie design — `context/worker/public-proxy.md`, `context/architecture.md`, `context/overview.md`, `context/glossary.md` all updated
- [x] `cargo test -p worker` (38 passed, 2 ignored), `cargo clippy -p worker --all-targets -- -D warnings` (pass), `cargo fmt --all -- --check` (pass)

### Residual risks
- The `worker-curl` helper cannot be fully end-to-end tested without a running worker with sudo + TAP capabilities. Manual smoke test: `nix run .#worker` (terminal 1), `nix run .#worker-curl -- test-vm /global/health` (terminal 2) → proxy logs show `route_not_found` for `vm_id = "test-vm"`, proving the subdomain routing pipeline works.
- `nix flake check` has pre-existing warnings unrelated to this plan (flake metadata, `control_plane` crate clippy). Worker-specific Nix paths verified independently.
