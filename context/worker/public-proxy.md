# Worker Public Proxy

## Current scope delivered
The proxy now uses **subdomain-based routing** (`https://<vm_id>.<base_domain>/<path>`) for VM traffic, with **cookie-bootstrap auth** for browser-based access.

### Routing (T01/T02 from original path-based plan, now subdomain)
- Route extraction: parses `Host: <vm_id>.<base_domain>` to identify the target VM.
- Authentication: accepts JWT via `Authorization: Bearer <jwt>` header or `Cookie: pcr_session=<jwt>`.
- Bootstrap endpoint at `/_pcr/auth?token=<jwt>&next=<path>` sets a per-VM scoped cookie and redirects.
- Logout endpoint at `/_pcr/logout` clears the session cookie.
- Signature verification: validates token using HS256 with `proxy.jwt_hs256_secret`.
- VM resolution: looks up VM IP from worker registry via `extract_vm_proxy_route`.
- Upstream construction: dispatches to `http://{vm_ip}:4096` preserving the original path and query.
- Timeout handling: applies configurable `proxy.upstream_request_timeout_millis` around upstream dispatch.
- Response mode: streams upstream response body without buffering (SSE compatible).

### Config contract
`worker/src/config.rs`
- `rpc_listen_addr` (private worker RPC)
- `proxy.public_listen_addr` (public HTTPS proxy)
- `proxy.tls_cert_path`
- `proxy.tls_key_path`
- `proxy.base_domain` (used for Host header parsing and cookie Domain scoping)
- `proxy.jwt_hs256_secret`
- `proxy.upstream_request_timeout_millis` (optional; enforced as per-request timeout)

### Startup validation rules
- RPC and proxy listener addresses must differ.
- TLS cert path must exist.
- TLS key path must exist.
- `base_domain` must be non-empty.
- JWT HS256 secret must be non-empty after trim.
- If `base_domain` does not start with `.`, an empty subdomain prefix (e.g. `Host: <base_domain>`) is treated as a missing VM subdomain (404).

### Error mapping
| Condition | Status |
|---|---|
| missing/invalid JWT | `401 Unauthorized` |
| valid JWT without VM authorization | `403 Forbidden` |
| unknown/absent VM id | `404 Not Found` |
| upstream URI build or transport failure | `502 Bad Gateway` |
| timeout-expired upstream dispatch | `504 Gateway Timeout` |

### Observability fields (per-request log)
- `vm_id`
- `upstream`
- `status`
- `latency_ms`
- `auth_result` — `"bearer"`, `"cookie"`, `"missing_or_invalid_token"`, `"forbidden_for_vm"`, `"route_not_found"`

### Module layout
- `worker/src/proxy/core.rs` — route extraction, auth, upstream dispatch, logging
- `worker/src/proxy/tls.rs` — TLS listener, cert loading, hyper server loop
- `worker/src/proxy/mod.rs` — re-exports

### Dev tooling
- `nix/flake/apps.nix` provides `nix run .#worker` (boots with auto-generated wildcard TLS cert), `nix run .#worker-curl -- <vm-id> <path>` (subdomain-based curl wrapper), and `nix run .#worker-token -- <vm-id>` (JWT minting).
- Dev cert uses CN `*.worker.local` with SAN `DNS:*.worker.local, DNS:worker.local`.
- Dev config sets `base_domain = "worker.local"`.
