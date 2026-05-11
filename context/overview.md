# Overview

## Current focus
- `worker` now includes a **TLS-terminated path-based reverse proxy** for VM routes at `/vm/{id}/...`.
- T02 implemented route extraction, VM registry lookup, URI rewrite to VM OpenCode upstream (`http://{vm_ip}:4096`), and response streaming passthrough.
- T03 implemented proxy authn/authz gate using `Authorization: Bearer <jwt>` with HS256 verification and `vm_ids` membership checks before forwarding.
- T04 implemented TLS listener wiring so proxy HTTPS and existing RPC listener run concurrently in the same worker process.
- T05 implemented proxy request observability (`vm_id`, `upstream`, `status`, `latency_ms`, `auth_result`) and deterministic gateway timeout/error translation via configured upstream request timeout.

## Worker proxy status
- Config supports distinct listeners:
  - `rpc_listen_addr` (private worker RPC)
  - `proxy.public_listen_addr` (public TLS proxy)
- Proxy config includes TLS cert/key paths, JWT HS256 secret, and optional upstream timeout knobs.
- Startup fails fast on invalid proxy config (missing TLS files, empty JWT secret, listener conflict).
- Runtime proxy core behavior now available in `worker/src/proxy/core.rs`:
  - extracts vm id from `/vm/{id}/...`
  - requires `Authorization: Bearer <jwt>` and verifies HS256 signature using `proxy.jwt_hs256_secret`
  - enforces VM-scoped authorization by checking `{id}` membership in JWT `vm_ids` claim
  - strips `/vm/{id}` prefix before upstream dispatch
  - resolves VM IP from worker registry and forwards to `http://{vm_ip}:4096`
  - applies optional `proxy.timeouts.upstream_request_timeout_millis` during upstream dispatch
  - streams upstream HTTP/SSE response body without buffering
  - returns `401` for missing/invalid token, `403` for token lacking VM access, `404` for unknown VM id, and deterministic gateway class errors (`502` transport/build failures, `504` timeout)
  - emits per-request logs with `vm_id`, `upstream`, `status`, `latency_ms`, and `auth_result`

## See also
- [architecture.md](./architecture.md)
- [worker/public-proxy.md](./worker/public-proxy.md)
