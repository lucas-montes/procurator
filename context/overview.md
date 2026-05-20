# Overview

## Current focus
- `worker` now includes a **TLS-terminated subdomain-based reverse proxy** for VM routes at `https://<vm_id>.<base_domain>/...`.
- Route extraction from `Host` header, VM registry lookup, URI rewrite to VM OpenCode upstream (`http://{vm_ip}:4096`), and response streaming passthrough.
- Proxy authn/authz gate using `Authorization: Bearer <jwt>` header or `Cookie: pcr_session=<jwt>`, with HS256 verification and `vm_id` membership check before forwarding.
- Cookie-bootstrap auth at `/_pcr/auth?token=<jwt>` for browser-based access.
- TLS listener wiring so proxy HTTPS and existing RPC listener run concurrently in the same worker process.
- Proxy request observability (`vm_id`, `upstream`, `status`, `latency_ms`, `auth_result`) and deterministic gateway timeout/error translation via configured upstream request timeout.
- `auth_result` log field distinguishes `"bearer"` (header auth) from `"cookie"` (bootstrap cookie auth).

## Worker proxy status
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
- [worker/public-proxy.md](./worker/public-proxy.md)
