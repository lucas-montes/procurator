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

<<<<<<< HEAD
## Key Components
- **CLI (`cli/`)**: Command-line interface with subcommands for VCS, stack management, and agent workspaces
- **repo_outils (`repo_outils/`)**: Git and Nix utility functions used across the project
- **autonix (`autonix/`)**: Nix-related automation and analysis tools
- **repohub (`repohub/`)**: Repository hub service for project discovery
- **control_plane (`control_plane/`)**: Central coordination service
- **worker (`worker/`)**: Background job execution service
- **ci_service (`ci_service/`)**: Continuous integration service
- **cache (`cache/`)**: Caching service for build artifacts

## CLI Namespaces (pcr)
- `pcr vcs`: Version control operations (project/repo management)
- `pcr stack`: Local project stack lifecycle
- `pcr agents`: Workspace management for AI agents
- `pcr init`: Initialize workspace

## Technology Stack
- **Language**: Rust (edition 2024)
- **Build tool**: Cargo with workspace structure
- **Git operations**: git2 (Rust bindings with vendored libgit2)
- **Nix integration**: Flake-based configurations
- **Async runtime**: Tokio
- **CLI framework**: Clap with derive macros

## Current State
- VCS commands are implemented for `pcr vcs repo`, `pcr vcs project`, and `pcr vcs agent`.
- Project operations support submodules with selective `--repos` / `--exclude` filtering.
- Repo push supports optional Nix cache upload when cache URL is configured in flake settings.
- Branch operations exist for repo and project flows (`project branch` remains a guided/manual stub path).
- Agent workspace prepare/list commands are implemented with co-located workspaces at `<project-dir>/agents/<branch>/`.
- Local clone acceleration is enabled through bare mirror cache references at `~/.cache/procurator/repo-cache/`.
- VCS command handlers emit execution timing logs (e.g. `... completed in ...`) for operational visibility.
- Repohub now defines a forge-agnostic signal ingestion boundary via `application::ports::ForgeSignalPort`, returning normalized domain signal types and keeping provider DTOs inside adapters.
- Repohub persists normalized signals in `normalized_signals` (upsert keyed by `(repository_id, signal_type, source_key)`) and weekly metric snapshots in `weekly_metric_snapshots` (upsert keyed by `(repository_id, week_start_utc, metric_version)`) with single-repo rolling-window retrieval.
- Repohub computes weekly DORA/productivity snapshots through `domain::metrics::WeeklyMetricEngine` with deterministic ordering and edge-case contracts (7-day anchored window, `[start,end)` timestamp inclusion, integer-second medians, and deterministic CFR/MTTR matching semantics).
- Repohub exposes a DORA metrics read API at `/{username}/{project}/{repo}/dora/metrics?week=` returning JSON array of `WeeklyMetricSnapshotRow`, backed by a periodic background task calling `RefreshOrchestrator::trigger_refresh` on a configurable interval.
- `ForgeSignalPort` trait requires `Send + Sync` so `RefreshOrchestrator` can be used across `tokio::spawn` boundaries.
- Repohub renders a minimal read-only DORA dashboard at `/{username}/{project}/{repo}/dora` with week-picker dropdown, grouped metric tables, and Chart.js trend charts over all available weeks.
=======
## See also
- [architecture.md](./architecture.md)
- [worker/public-proxy.md](./worker/public-proxy.md)
>>>>>>> master
