# Patterns

## Fail-fast startup config validation
- Parse config once at startup.
- Validate required file paths and secrets before long-running services boot.
- Exit with explicit error when invalid.

Applied in `worker/src/config.rs` for proxy contract validation.

## Boundary-first module introduction
- Define interfaces/contracts first.
- Defer concrete network/runtime behavior to follow-up tasks.

Applied in `worker/src/proxy/interfaces.rs` for T01.

## Path-prefix reverse proxy rewrite
- Accept only canonical public VM route prefix `/vm/{id}`.
- Authenticate with `Authorization: Bearer <jwt>` and verify HS256 signature using worker proxy secret.
- Authorize by requiring requested `{id}` to be present in JWT `vm_ids` claim.
- Resolve `{id}` to current VM IP from worker registry at request time.
- Rewrite upstream URI by stripping `/vm/{id}` and preserving remaining path/query.
- Dispatch with Hyper client and pass through response/body stream for HTTP + SSE compatibility.
- Apply optional per-request upstream timeout from `proxy.timeouts.upstream_request_timeout_millis`.
- Map missing/invalid token to `401`, valid token without VM access to `403`, unknown VM to `404`, upstream transport/build failures to `502`, and timeout-expired dispatch to `504`.
- Emit per-request observability fields (`vm_id`, `upstream`, `status`, `latency_ms`, `auth_result`) for both success and mapped failure paths.

Applied in `worker/src/proxy/core.rs` for T02/T03/T05.
