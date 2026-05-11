# Worker Public Proxy

## Current scope delivered
- T01: added worker proxy config contract and startup validation rules.
- T02: added path-based reverse proxy core for `/vm/{id}/...` routing.
- T03: added JWT authn/authz enforcement (`Authorization: Bearer`, HS256 verification, `vm_ids` claim membership).
- T04: added TLS listener serving and process lifecycle wiring so proxy HTTPS + worker RPC run concurrently.
- T05: added proxy observability logging, runtime timeout config usage, and deterministic gateway error mapping while preserving SSE streaming passthrough.

## Config contract
`worker/src/config.rs`
- `rpc_listen_addr` (with `listen_addr` alias for compatibility)
- `proxy.public_listen_addr`
- `proxy.tls_cert_path`
- `proxy.tls_key_path`
- `proxy.jwt_hs256_secret`
- `proxy.timeouts.upstream_connect_timeout_millis` (optional; reserved/not yet wired)
- `proxy.timeouts.upstream_request_timeout_millis` (optional; enforced as per-request upstream dispatch timeout)

## Startup validation rules
- RPC and proxy listener addresses must differ.
- TLS cert path must exist.
- TLS key path must exist.
- JWT HS256 secret must be non-empty after trim.

## Proxy runtime behavior (T02/T03/T05)
`worker/src/proxy/core.rs`
- Route extraction: accepts `/vm/{id}/...` and extracts `{id}`.
- Authentication: requires `Authorization: Bearer <jwt>`.
- Signature verification: validates token using HS256 with `proxy.jwt_hs256_secret`.
- Authorization: requires extracted `{id}` to be present in JWT `vm_ids` claim.
- Prefix stripping: upstream path rewrites from `/vm/{id}/x` to `/x` (query preserved).
- VM resolution: lookup `{id}` in worker registry and read current VM IP from handle.
- Upstream construction: dispatches to `http://{vm_ip}:4096`.
- Timeout handling: applies optional `proxy.timeouts.upstream_request_timeout_millis` around upstream dispatch.
- Response mode: returns upstream response directly, including streaming bodies suitable for SSE (no buffering layer introduced).
- Error mapping:
  - missing/invalid JWT => `401 Unauthorized`
  - valid JWT without VM authorization => `403 Forbidden`
  - unknown/absent VM id => `404 Not Found`
  - upstream URI build or transport failure => `502 Bad Gateway`
  - timeout-expired upstream dispatch => `504 Gateway Timeout`
- Observability fields emitted per request:
  - `vm_id`
  - `upstream`
  - `status`
  - `latency_ms`
  - `auth_result`

## Module boundaries
`worker/src/proxy/interfaces.rs`
- `VmTargetResolver`
- `TokenAuthorizer`
- `ProxyRequestRewriter`
- `ProxyUpstreamTransport`
- `ProxyRequestContext`, `AuthResult`, `ProxyUpstreamTarget`

## Validation status snapshot
- All proxy implementation tasks complete (T01–T05).
- T06 validation pass: `cargo clippy -p worker --all-targets -- -D warnings` passes, `cargo test -p worker` passes (31/31, 2 ignored for CAP_NET_ADMIN), `cargo fmt --all -- --check` passes.
- Pre-existing lint debt in `ch/tap.rs`, `server.rs`, `proxy/core.rs`, and `proxy/tls.rs` resolved as part of T06.
