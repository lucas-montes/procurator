# Repohub — Project, Repository & DORA Metrics Management

A web service for managing users, projects, Git repositories, and DORA metrics.
Built with Axum, Askama templates, and SQLite. Serves a GitHub-inspired HTML
interface and JSON API.

## Quick Start

### 1. Configuration

`repohub` is configured through `repohub::Config`. The binary uses the defaults
in code, and a hosting process can populate the fields from environment
variables or another config source before startup.

| Variable | Default | Purpose |
|----------|---------|---------|
| `DATABASE_URL` | `../repohub.db` | SQLite database path |
| `BIND_ADDRESS` | `0.0.0.0:3001` | Server listen address |
| `DOMAIN` | `homelab` | Domain name for links |
| `REPOS_BASE_PATH` | `git-server` | Base path for bare Git repos |
| `GITHUB_APP_ID` | `None` | GitHub App ID used for installation auth |
| `GITHUB_APP_PRIVATE_KEY_PEM` | `None` | PEM-encoded private key used to sign GitHub App JWTs |
| `GITHUB_WEBHOOK_SECRET` | `None` | Shared secret for validating GitHub App webhook requests |
| `GITHUB_OAUTH_CLIENT_ID` | `""` | Legacy GitHub OAuth App client ID (empty = legacy flow disabled) |
| `GITHUB_OAUTH_CLIENT_SECRET` | `""` | Legacy GitHub OAuth App client secret (empty = legacy flow disabled) |
| `GITHUB_OAUTH_REDIRECT_URL` | `""` | Legacy OAuth callback URL, e.g. `http://host:3001/auth/github/callback` |
| `GITHUB_DORA_INTERVAL_SECONDS` | `3600` | Background refresh interval (seconds) |
| `GITHUB_DORA_INCIDENT_LABEL_PATTERNS` | `.*incident.*` | Comma-separated label patterns for incidents |

### 2. Run

```bash
# Development (from the workspace root)
cargo run -p repohub

# Or set up a .env file and use:
cargo run -p repohub
```

The database and tables are created automatically on first startup.

> **Note:** DORA metrics are automatically computed for **all repositories** in the
> database with a GitHub `git_url` (e.g. `https://github.com/owner/repo.git`).
> Repositories hosted elsewhere are skipped. No per-repo configuration needed.

### 3. GitHub App setup

Repohub now uses a **GitHub App** for server-side GitHub access, repo listing,
and webhook validation. GitHub Apps provide installation-scoped, short-lived
tokens and are the recommended path for new setups.

##### Step 1: Register a GitHub App

1. Go to **GitHub Settings → Developer settings → [GitHub Apps](https://github.com/settings/apps)** → **New GitHub App**
2. Fill in the form:
  - **GitHub App name:** `Repohub (local)` or any name you prefer
  - **Homepage URL:** `http://localhost:3001` (adjust to your host/port)
  - **Webhook URL:** `http://localhost:3001/github/webhook`
  - **Callback URL:** not required for the GitHub App flow; only set one if you still use the legacy OAuth flow
  - **Webhook secret:** choose a strong secret and copy it into `GITHUB_WEBHOOK_SECRET`
3. Grant the minimum permissions needed for your workflow.
  - For repo import and metadata, start with repository metadata read access.
  - Add more permissions only if your workflows need them.
4. Subscribe to the webhook events you plan to use, then generate and download the private key PEM.
5. Install the app on the user/org and repositories you want `repohub` to access.

##### Step 2: Provide the config values

Populate `repohub::Config` before starting the service:

```rust
use repohub::Config;

let config = Config {
   github_app_id: Some(12345),
   github_app_private_key_pem: Some(include_str!("/path/to/github-app.pem").to_string()),
   github_webhook_secret: Some("your-webhook-secret".to_string()),
   ..Config::default()
};
```

##### Step 3: Start Repohub

Start the service with the config populated in your hosting process. If you use
environment variables, map them to the fields above before constructing
`Config`.

##### Step 4: Validate the GitHub App flow

1. Start `repohub` and confirm it boots with the GitHub App fields populated.
2. Verify the GitHub App installation can be used to list repos through
  `/{username}/github/repos`.
3. If you expose webhooks locally, send a test webhook to
  `http://localhost:3001/github/webhook` and confirm the signature matches
  `github_webhook_secret`.

Legacy OAuth and PAT-based account connection still exist in the UI, but they
are not the recommended setup for new deployments.

### 4. Manual testing checklist

Use these steps to verify the GitHub App flow locally:

1. Create a GitHub App at `https://github.com/settings/apps` with these values:
  - **Homepage URL:** `http://localhost:3001`
  - **Webhook URL:** `http://localhost:3001/github/webhook`
  - **Callback URL:** only needed for the legacy OAuth flow, not for the GitHub App flow
  - **Webhook secret:** copy it into `github_webhook_secret`
  - **Permissions:** start with repository metadata read access and add more only if your workflow needs it
  - **Events:** subscribe to the webhook events you want to test
2. Copy the App ID and download the private key PEM.
3. Fill `repohub::Config` before startup:

```rust
use repohub::Config;

let config = Config {
   github_app_id: Some(12345),
   github_app_private_key_pem: Some(include_str!("/path/to/github-app.pem").to_string()),
   github_webhook_secret: Some("your-webhook-secret".to_string()),
   ..Config::default()
};
```

4. Start the service:

```bash
cargo run -p repohub
```

5. Verify the GitHub App auth path:
  - Open a user page and confirm `/{username}/github/repos` returns repositories for the installed app
  - Open the repo import modal and confirm the GitHub list loads
6. Verify webhook handling:
  - Send a test webhook to `http://localhost:3001/github/webhook`
  - Confirm the request is accepted when the signature matches `github_webhook_secret`
7. If you still use the legacy OAuth/PAT flow for existing users, confirm that path still works separately

If repo listing is empty or returns `403`, check the app permissions and that the app is installed on the target repository.

## Available Endpoints

All routes are mounted at the root. Repository context follows the path pattern
`/{username}/{project}/{repo}`.

### GitHub-like UI (HTML + JSON)

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/` | Home — list all users |
| `POST` | `/users` | Create a new user (optionally with `github_token`) |
| `GET` | `/{username}` | View user profile + their projects |
| `GET` | `/{username}/auth/github` | Start legacy GitHub OAuth flow (redirect to GitHub) |
| `GET` | `/auth/github/callback` | Legacy GitHub OAuth callback handler |
| `GET` | `/{username}/github/status` | Check GitHub connection status (JSON) |
| `GET` | `/{username}/github/repos` | List repositories available to the configured GitHub auth setup (JSON) |
| `POST` | `/{username}/github-token` | Update GitHub PAT (legacy) |
| `DELETE` | `/{username}/github-token` | Disconnect GitHub account |
| `POST` | `/{username}/projects` | Create a new project |
| `GET` | `/{username}/{project}` | View project details |
| `POST` | `/{username}/{project}/repositories` | Create a repository |
| `GET` | `/{username}/{project}/{repo}` | View repository dashboard |
| `GET` | `/{username}/{project}/{repo}/builds/{id}` | View build details |
| `GET` | `/{username}/{project}/{repo}/flake` | View Nix flake metadata |
| `GET` | `/{username}/{project}/testing` | Testing page |
| `GET` | `/{username}/{project}/configuration` | View project configuration |
| `POST` | `/{username}/{project}/configuration` | Save project configuration |
| `GET` | `/{username}/{project}/agents` | Agent workspace management |
| `GET` | `/{username}/{project}/documentation` | Project documentation |
| `GET` | `/{username}/{project}/stats` | Project statistics |
| `GET` | `/{username}/{project}/milestones` | Project milestones |

### Gerrit Review System (JSON API + HTML UI)

All Gerrit routes are mounted under `/{username}/{project}/{repo}/`.

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/gerrit/{username}/{project}/{repo}/changes` | List changes (JSON) |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes` | Create a change |
| `GET` | `/gerrit/{username}/{project}/{repo}/changes/ui` | List changes (HTML) |
| `GET` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/ui` | Change detail (HTML) |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/patchsets` | Upload a patch set |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/votes` | Add/update a label vote |
| `GET` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/submit-readiness` | Evaluate submit checks |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/submit` | Submit a change |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/abandon` | Abandon a change |
| `POST` | `/gerrit/{username}/{project}/{repo}/changes/{change_id}/restore` | Restore an abandoned change |

To access the Gerrit UI from any repository page, click **"Open Gerrit Changes"**.

### DORA Metrics Dashboard (HTML + JSON API)

DORA endpoints are mounted under `/{username}/{project}/{repo}/dora/`.

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/{username}/{project}/{repo}/dora` | DORA dashboard (HTML with Chart.js graphs) |
| `GET` | `/{username}/{project}/{repo}/dora/metrics` | DORA metrics JSON API |

**Query parameters for `/dora/metrics`:**
- `?week=2026-05-04` — filter snapshots by week start date (ISO format)
  When omitted, all available snapshots (up to 52 weeks) are returned.

**Dashboard features:**
- Week picker dropdown to navigate between computed weeks
- Four grouped metric sections: Counts, Cycle Stages, DORA Rates, Medians
- Four Chart.js trend charts (counts, durations, cycle stages, CFR) across all available weeks
- Human-readable duration formatting (e.g. "3d 12h", "1h 30m", "45m 20s")
- CFR displayed as percentage (e.g. "5.0%")
- Empty-state message when no data has been computed yet

### DORA Background Refresh

A background task starts automatically on server startup. On each tick it:

1. Queries **all repositories** from the database
2. For each repository with a GitHub `git_url` (e.g. `https://github.com/owner/repo.git`),
   parses the owner and repo name
3. Looks up the project owner's **Personal Access Token (PAT)** from the database
4. Skips the repository if no token is configured for the owner
5. Fetches all raw signals (PRs, reviews, commits, deployments, issues) using the PAT
6. Persists them as normalized signals
7. Detects week windows from signal timestamps
8. Computes all DORA and productivity metrics for each window
9. Stores metric snapshots

The first refresh runs immediately on startup, then repeats on the configured
interval (`GITHUB_DORA_INTERVAL_SECONDS`, default 3600s). Failures are logged
per-repository and retried on the next tick — one failing repository does not
block others.

## Metrics Computed

The weekly metric computation engine produces 22 fields per week:

**Counts:** PRs Opened, Merge Frequency, Deployment Frequency, Commits,
Code Changes, Total Reviews, Merged Without Review, Handoffs

**Cycle Stages (median durations):** Coding, Pickup, Review, Deploy

**DORA:** Deployment Frequency, Change Failure Rate (CFR), MTTR,
Lead Time for Changes

**Medians (durations/sizes):** PR Size, Review Depth, Review Time,
PR Pickup Time, Time to Merge, Deployment Time

Change Failure Rate uses a **mixed model**: a deployment counts as failed if its
state is not `success`, or if an incident issue (identified by label pattern) was
opened during that deployment's timeframe. Recovery is the nearest-forward
successful deployment or closed incident.

## Library-First Design

Structured as a library (`repohub::*`) with a thin binary (`main.rs`).
The library can be embedded into another service or run standalone.

Re-exports and state types:
- `repohub::Config` — application configuration
- `repohub::Database` — SQLite database handle
- `repohub::GithubAppState` / `repohub::github_routes()` — GitHub-like routes
- `repohub::GerritAppState` / `repohub::gerrit_routes()` — Gerrit review routes
- `repohub::DoraAppState` / `repohub::dora_routes()` — DORA metrics routes
- `repohub::RefreshOrchestrator` — async pipeline orchestrator
- `repohub::RepositoryService` — Git repository service
