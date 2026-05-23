# Repohub Forge Ports (T04)

Application boundary for forge-agnostic ingestion in `repohub/src/application/ports.rs`.

## Purpose
- Keep application contracts provider-neutral.
- Return only normalized domain signals from adapters.
- Prevent provider DTO leakage beyond adapter internals.

## Trait Constraints
- `ForgeSignalPort: Send + Sync` — required so `RefreshOrchestrator` (which holds `Box<dyn ForgeSignalPort>`) can be moved across `tokio::spawn` boundaries.

## Core Contracts
- `ForgeSignalPort`
  - `fetch_pull_requests(target)` -> `Vec<NormalizedPullRequest>`
  - `fetch_reviews(target)` -> `Vec<NormalizedReview>`
  - `fetch_commits(target)` -> `Vec<NormalizedCommit>`
  - `fetch_deployments(target)` -> `Vec<NormalizedDeployment>`
  - `fetch_issues(target)` -> `Vec<NormalizedIssue>`
  - `fetch_all(target)` -> `NormalizedSignalBatch`
- `ForgeRepositoryTarget`
  - `repository_id: i64`
  - `owner: String`
  - `name: String`
- `ForgeError`
  - `InvalidInput`
  - `Authentication`
  - `Upstream`
  - `Transform`

## GitHub Adapter Conformance
- `adapters/github/client.rs` implements `ForgeSignalPort`.
- GitHub-specific DTOs remain in `adapters/github/dto.rs` and are normalized in adapter code before returning through port methods.
- Target mismatch (`owner/name`) is rejected as `ForgeError::InvalidInput`.

## Notes
- Existing `application/github/ports.rs` remains for legacy CRUD-style repository/project/user operations and is separate from signal ingestion contracts.

## Adapter Extension Guidance (Adding a New Forge)

This section documents the contract a new forge adapter (e.g. GitLab, Bitbucket, Azure DevOps) must satisfy.

### Implementation Checklist

1. **Create adapter module** under `repohub/src/adapters/<forge>/`.
   - Minimum files: `mod.rs` (re-exports), `client.rs` (implements `ForgeSignalPort`), `dto.rs` (provider-specific DTOs, module-private).
2. **Implement `ForgeSignalPort`** for the new adapter struct.
   - All six methods: `fetch_pull_requests`, `fetch_reviews`, `fetch_commits`, `fetch_deployments`, `fetch_issues`, `fetch_all`.
   - Return types use **only** `Normalized*` domain types — no provider DTOs leak past the adapter boundary.
3. **Keep DTOs private** — `dto.rs` types must not be `pub` beyond the adapter module. Normalization functions in `client.rs` (or a `transform.rs`) convert DTOs to domain types.
4. **Implement `Send + Sync`** — the adapter struct (and any held clients/sessions) must satisfy these supertraits so `RefreshOrchestrator` can cross `tokio::spawn` boundaries.
5. **Map errors to `ForgeError`**:
   - `InvalidInput` — bad target parameters (e.g., owner/name not found)
   - `Authentication` — credential/token failures
   - `Upstream` — provider API errors (rate limits, network, 5xx)
   - `Transform` — data parsing/normalization failures
6. **Respect `ForgeRepositoryTarget`** — use `repository_id`, `owner`, `name` to scope calls. Reject mismatched targets.

### Minimum Viable Adapter

A new adapter needs only the `fetch_all` method to be usable (the orchestrator calls only `fetch_all`). Individual fetch methods exist for testability and partial re-fetch. A valid implementation pattern:

```
┌──────────────────────────────┐
│  GithubClient (example)      │
│  ├── http_client: Client     │
│  ├── app_id: u64             │
│  ├── installation_id: u64   │
│  └── private_key: Vec<u8>   │
├── ForgeSignalPort impl       │
│  ├── fetch_all(target)       │
│  │   ├── fetch_pull_requests │
│  │   ├── fetch_reviews       │
│  │   ├── fetch_commits       │
│  │   ├── fetch_deployments   │
│  │   ├── fetch_issues        │
│  │   └── assemble batch      │
│  └── …                       │
└──────────────────────────────┘
```

### Testing Pattern

- Use the same mock port strategy as `RefreshOrchestrator` tests: implement `ForgeSignalPort` on a test struct that returns controlled `Normalized*` vectors.
- Provider integration tests should verify DTO deserialization and normalization produce domain types matching fixture expectations.
- No new forge adapter is required in v1; extension readiness is the only goal.

### Common Pitfalls

- **Leaking provider types** — if a `Normalized*` return path contains `Option<ProviderSpecificEnum>`, the port boundary is broken. Use only domain types.
- **Missing `Send + Sync`** — the trait requires both; if the adapter holds a non-`Sync` HTTP client, `tokio::spawn` will reject the orchestrator.
- **Incorrect timestamp handling** — all timestamps in domain types must be strict RFC3339. The engine relies on `DateTime<Utc>` comparisons for window filtering.
- **Environment normalization** — the production environment check (`is_production()`) must be implemented per-forge, e.g. via environment-name matching (GitHub: `"production"` environment or `production_environment` flag) or explicit configuration.
- **Incident classification** — incident issue detection relies on configurable label regex patterns. Adapters must return all issue labels as raw strings; the engine handles pattern matching.
