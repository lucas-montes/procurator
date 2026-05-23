# Repohub Normalized Signals (T03)

Canonical normalized signal model for forge-ingested data in `repohub/src/domain/signals/`.

The forge-agnostic application boundary that serves these types is defined in `repohub/src/application/ports.rs` (`ForgeSignalPort`).

## Scope
- Pull requests, reviews, commits, deployments, and incident issues normalized from GitHub DTOs.
- Timestamp semantics for metric computations.
- PR↔deploy↔incident identity/link keys.
- Mixed-model failure/recovery projection for CFR/MTTR.

## Core Types
- `NormalizedPullRequest`
  - State enum: `PullRequestState` (`Open|Closed|Merged|Unknown`)
  - Identity: `(repository_id, number)` plus commit-sha link keys.
- `NormalizedReview`
  - State enum: `ReviewState` (`Approved|ChangesRequested|Commented|Dismissed|Unknown`)
- `NormalizedCommit`
  - Coding stage semantics via `coding_started_at()` (author timestamp).
- `NormalizedDeployment`
  - State enum: `DeploymentState`
  - Environment enum: `DeploymentEnvironment`
  - Production normalization via `is_production` and environment fallback.
- `NormalizedIssue`
  - State enum: `IssueState`
  - Incident role enum: `IncidentRole` (`FailureSignal|RecoverySignal`)

## Timestamp Semantics
- PR opened: `NormalizedPullRequest::opened_at()` → `created_at`
- PR merged: `merged_timestamp()` → `merged_at`
- PR terminal: `terminal_at()` → `merged_at` else `closed_at`
- Review submitted: `submitted_timestamp()` → `submitted_at`
- Commit coding start: `coding_started_at()` → `authored_at`
- Commit weekly bucket: `committed_timestamp()` → `committed_at`
- Deployment event: `deployed_at()` → `created_at`
- Incident opened/closed: `opened_at()` / `closed_timestamp()`

## Identity and Linking Strategy (PR↔Deploy↔Incident)
Defined in `signals/linking.rs`.

- `PullRequestIdentity { repository_id, number }`
- `SignalLinkKey`
  - `PullRequest(PullRequestIdentity)`
  - `CommitSha(String)`
  - `Repository(i64)`

Entity link key emission:
- PR: `PullRequestIdentity`, `head_sha`, optional `merge_commit_sha`, repository key
- Deployment: deployed `sha`, repository key
- Incident issue: repository key (v1 repository-scoped incident correlation)

## Mixed Failure/Recovery Semantics
Defined in `signals/failure.rs`.

- Canonical event: `FailureSignal`
  - `source: FailureSignalSource` (`Deployment|Incident`)
  - `occurred_at`
  - `is_recovery`
- Deployment projection:
  - only production deployments emit a signal
  - success => recovery signal
  - non-success => failure signal
- Incident projection:
  - incident-labelled open issue => failure signal
  - incident-labelled closed issue => recovery signal at `closed_at`

This model supports plan-defined mixed behavior: first failure signal (deployment or incident) and earliest valid recovery signal.

## Transform Layer
`signals/transform.rs` provides normalization from GitHub DTOs:
- `normalize_pull_request`
- `normalize_review`
- `normalize_commit`
- `normalize_deployment`
- `normalize_issue`

All timestamp parsing is strict RFC3339 via `TransformError::InvalidTimestamp`.

## Tests
- Model semantics tests in:
  - `pull_request.rs`
  - `review.rs`
  - `deployment.rs`
  - `issue.rs`
  - `failure.rs`
- Representative fixture transformation tests in:
  - `transform.rs`
