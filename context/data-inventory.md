# Data Inventory (GitHub Ingestion, Current State)

Canonical field inventory for the `forge-dora-dashboard-v1` flow after T03 normalization work.

## DTO Coverage (ingested from GitHub)

Source: `repohub/src/adapters/github/dto.rs`

### Pull Requests (`GithubPullRequest`)
- Identity/lifecycle: `id`, `number`, `state`, `created_at`, `updated_at`, `closed_at`, `merged_at`
- Author/content: `title`, `user`, `author_association`, `draft`
- Size/change: `additions`, `deletions`, `changed_files`
- Branch/linking: `head.{sha,ref}`, `base.{sha,ref}`, `merge_commit_sha`

### Reviews (`GithubReview`)
- `id`, `user`, `state`, `submitted_at`, `body`, `commit_id`

### Commits (`GithubCommit`)
- `sha`
- `commit.author{date,...}`, `commit.committer{date,...}`, `commit.message`
- `author`, `committer`
- `stats{additions,deletions,total}`

### Deployments (`GithubDeployment`)
- Identity/state: `id`, `sha`, `state`, `created_at`, `updated_at`
- Context: `ref`, `task`, `payload`, `environment`, `description`
- Actor/flags: `creator`, `production_environment`

### Issues (`GithubIssue`)
- Identity/lifecycle: `id`, `number`, `state`, `created_at`, `updated_at`, `closed_at`
- Author/content: `title`, `user`, `labels`
- Classification/linking: `pull_request` marker (PR-vs-issue), `assignee`, `milestone`

## Normalized Mapping Readiness

Source: `repohub/src/domain/signals/transform.rs`

All required signal families are normalized with strict RFC3339 timestamp parsing:
- Pull request
- Review
- Commit
- Deployment
- Issue

Normalization now includes link-relevant fields needed for downstream correlation:
- PR `head_sha`, `merge_commit_sha`, repository identity
- Deployment `sha` + repository identity
- Incident issue repository identity

## Remaining Data Constraints (Not Missing Fields)

No T02 field-level DTO gaps remain for T03 scope.

Open constraints now are semantic/policy, not ingestion availability:
- Incident classification depends on configured label patterns.
- Production environment semantics depend on `production_environment` flag or environment-name fallback.
- Cross-signal correlation quality depends on commit-sha continuity between PR and deployment events.
