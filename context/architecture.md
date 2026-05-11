# Architecture

## Worker process boundaries (current state)
- Existing worker RPC server remains in `worker/src/server.rs`.
- Proxy runtime core + authn/authz gate exist in `worker/src/proxy/core.rs`, and public HTTPS listener serving + TLS wiring is implemented in `worker/src/proxy/tls.rs`.
- Proxy runtime applies optional upstream request timeout from config and emits request outcome logs for auth and upstream diagnostics.

```mermaid
flowchart LR
  Req[HTTP request /vm/{id}/...] --> Parse[extract_vm_proxy_route]
  Parse --> Auth[JWT authn/authz gate]
  Auth -->|missing/invalid token| Unauthorized[401]
  Auth -->|token lacks vm_ids membership| Forbidden[403]
  Auth -->|authorized| Lookup[lookup_vm_ip via Registry<Reader>]
  Lookup --> Rewrite[strip /vm/{id} prefix]
  Rewrite --> URI[build_upstream_uri http://{vm_ip}:4096]
  URI --> Dispatch[hyper Client request]
  Dispatch --> Timeout{request timeout configured?}
  Timeout -->|yes + elapsed| TimeoutErr[504 gateway timeout]
  Timeout -->|no or in-time| Stream[response passthrough body stream]

  Parse -->|invalid/unknown vm| NotFound[404]
  Dispatch -->|upstream transport/build failure| Gateway[502]
```

## Notes
- `worker/src/proxy/interfaces.rs` remains the contract boundary for upcoming abstraction tasks.
- T03 added JWT access control enforcement (`Authorization: Bearer`, HS256, `vm_ids` claim membership) ahead of VM lookup and upstream dispatch in `worker/src/proxy/core.rs`.
- Worker runtime now starts RPC listener (`worker/src/server.rs`), proxy HTTPS listener (`worker/src/proxy/tls.rs`), and supervisor loop concurrently on the same process-local Tokio runtime.
