# Overview

## Current focus
- Control plane ↔ worker **pull-based assignment** over the Cap'n Proto
  `Master` RPC surface is implemented end to end and proven by a cross-crate
  e2e test (`worker/src/e2e.rs`).
- `publishState` stores explicit generations (`commit`, `generation`,
  `intentHash`, `vmSpecs` with `id` + `targetWorker`); workers poll
  `getAssignment`, reconcile locally, and report `pushData` heartbeats;
  `getClusterStatus` serves generation/commit/convergence.
- Worker inventory is deterministic + Nix-configured (`cluster.vms`, CP
  `workers` list) with dynamic liveness from heartbeats; both binaries fail
  fast on invalid config.
- Worker runtime now supervises five concurrent tasks: RPC server, TLS proxy,
  VM supervisor, assignment poll loop, and heartbeat reporter.
- `worker` keeps its **TLS-terminated subdomain-based reverse proxy** for VM routes (unchanged behavior, see below).

## Pull assignment status
- Explicit placement only: every desired VM pins `id` + `targetWorker`;
  unknown targets and blank ids fail loudly, never silently.
- Node keeps generations and opaque spec bytes in memory (restart loses
  them); polls filter per worker; heartbeats merge observed state;
  convergence = observed generation + running set match per worker.
- Worker poll diffs desired keys vs registry into supervisor
  `Create`/`Delete` (no-op on unchanged generations, backoff on errors);
  heartbeat reports shape registry contents with `listVms`/`read` semantics.
- Transport seams for a future push mode exist on both sides
  (`AssignmentSource`, `WorkerDirectory`, `with_directory`) with no
  pull-mode users by design.
- Known limits: no CP persistence, no worker↔master auth (private net
  assumption), per-VM status rows stay empty, proxy JWT untouched.

## Worker proxy status
- Subdomain routing: `Host: <vm_id>.<base_domain>` extraction, VM registry
  lookup, URI rewrite to VM OpenCode upstream (`http://{vm_ip}:4096`), and
  response streaming passthrough.
- Proxy authn/authz gate using `Authorization: Bearer <jwt>` header or `Cookie: pcr_session=<jwt>`, with HS256 verification and `vm_id` membership check before forwarding.
- Cookie-bootstrap auth at `/_pcr/auth?token=<jwt>` for browser-based access.
- TLS listener wiring so proxy HTTPS and existing RPC listener run concurrently in the same worker process.
- Proxy request observability (`vm_id`, `upstream`, `status`, `latency_ms`, `auth_result`) and deterministic gateway timeout/error translation via configured upstream request timeout.
- `auth_result` log field distinguishes `"bearer"` (header auth) from `"cookie"` (bootstrap cookie auth).
- Config supports distinct listeners:
  - `rpc_listen_addr` (private worker RPC)
  - `proxy.public_listen_addr` (public TLS proxy)
- Proxy config includes TLS cert/key paths, `base_domain`, JWT HS256 secret, and optional upstream timeout knobs.
- Startup fails fast on invalid proxy config (missing TLS files, empty `base_domain`, empty JWT secret, listener conflict).
- Runtime proxy core behavior in `worker/src/proxy/core.rs`:
  - extracts vm id from `Host: <vm_id>.<base_domain>` header
  - accepts JWT via `Authorization: Bearer` header or `Cookie: pcr_session=<jwt>` cookie
  - verifies HS256 signature using `proxy.jwt_hs256_secret`
  - enforces VM-scoped authorization by checking `vm_id` in JWT claims
  - preserves original path and query in upstream dispatch
  - resolves VM IP from worker registry and forwards to `http://{vm_ip}:4096`
  - applies optional `proxy.upstream_request_timeout_millis` during upstream dispatch
  - streams upstream HTTP/SSE response body without buffering
  - returns `401` for missing/invalid token, `403` for token lacking VM access, `404` for unknown VM id, and deterministic gateway class errors (`502` transport/build failures, `504` timeout)
  - emits per-request logs with `vm_id`, `upstream`, `status`, `latency_ms`, and `auth_result`

## See also
- [architecture.md](./architecture.md)
- [control-plane/node-state.md](./control-plane/node-state.md)
- [worker/pull-reconcile.md](./worker/pull-reconcile.md)
- [worker/public-proxy.md](./worker/public-proxy.md)
