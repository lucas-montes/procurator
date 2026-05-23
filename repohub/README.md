# Repohub — Project, Repository & DORA Metrics Management

A web service for managing users, projects, Git repositories, and DORA metrics.
Built with Axum, Askama templates, and SQLite. Serves a GitHub-inspired HTML
interface and JSON API.

## Quick Start

### 1. Configuration

Configuration is read from the environment. All DORA-specific settings are optional
(the service runs without them, but DORA features will be disabled).

| Variable | Default | Purpose |
|----------|---------|---------|
| `DATABASE_URL` | `../repohub.db` | SQLite database path |
| `BIND_ADDRESS` | `0.0.0.0:3001` | Server listen address |
| `DOMAIN` | `homelab` | Domain name for links |
| `REPOS_BASE_PATH` | `git-server` | Base path for bare Git repos |
| `GITHUB_OAUTH_CLIENT_ID` | `""` | GitHub OAuth App client ID (empty = OAuth disabled) |
| `GITHUB_OAUTH_CLIENT_SECRET` | `""` | GitHub OAuth App client secret (empty = OAuth disabled) |
| `GITHUB_OAUTH_REDIRECT_URL` | `""` | OAuth callback URL, e.g. `http://host:3001/auth/github/callback` |
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

### 3. Connecting Your GitHub Account

Repohub supports **GitHub OAuth** as the recommended way to connect your GitHub account (replacing the older PAT-based flow).

#### Option A: GitHub OAuth (Recommended)

##### Step 1: Register a GitHub OAuth App

1. Go to **GitHub Settings → Developer settings → [OAuth Apps](https://github.com/settings/developers)** → **New OAuth App**
2. Fill in the form:
   - **Application name:** `Repohub (local)` (or any name you prefer)
   - **Homepage URL:** `http://localhost:3001` (adjust to your host/port)
   - **Authorization callback URL:** `http://localhost:3001/auth/github/callback` (must match `GITHUB_OAUTH_REDIRECT_URL` exactly)
3. Click **Register application**
4. On the next page, copy the **Client ID** and generate a **Client Secret** (copy it immediately — GitHub shows it only once)

##### Step 2: Configure Environment Variables

Set these environment variables before starting repohub:

| Variable | Default | Purpose |
|----------|---------|---------|
| `GITHUB_OAUTH_CLIENT_ID` | `""` | Client ID from your GitHub OAuth App |
| `GITHUB_OAUTH_CLIENT_SECRET` | `""` | Client Secret from your GitHub OAuth App |
| `GITHUB_OAUTH_REDIRECT_URL` | `""` | Full callback URL — must match the GitHub OAuth App's callback exactly, e.g. `http://localhost:3001/auth/github/callback` |

All three default to empty strings, which **disables OAuth** — the feature is opt-in.

##### Step 3: Run Repohub

```bash
GITHUB_OAUTH_CLIENT_ID="your_client_id" \
GITHUB_OAUTH_CLIENT_SECRET="your_client_secret" \
GITHUB_OAUTH_REDIRECT_URL="http://localhost:3001/auth/github/callback" \
cargo run -p repohub
```

Or place them in a `.env` file:

```bash
GITHUB_OAUTH_CLIENT_ID=your_client_id
GITHUB_OAUTH_CLIENT_SECRET=your_client_secret
GITHUB_OAUTH_REDIRECT_URL=http://localhost:3001/auth/github/callback
```

##### Step 4: Connect in the UI

1. Navigate to your user profile at `http://localhost:3001/{username}`
2. Click **"Connect to GitHub"** — you'll be redirected to GitHub's authorization page
3. Authorize the OAuth App
4. You'll be redirected back to your profile, now showing "Connected as **{github_login}**"

To disconnect, click the **"Disconnect"** button on your profile page.

#### Option B: Personal Access Token (Legacy)

Users may also set a **GitHub Personal Access Token (PAT)** directly. This method is deprecated but still functional for programmatic usage.

Repositories owned by users without a token are skipped during refresh.

## Available Endpoints

All routes are mounted at the root. Repository context follows the path pattern
`/{username}/{project}/{repo}`.

### GitHub-like UI (HTML + JSON)

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/` | Home — list all users |
| `POST` | `/users` | Create a new user (optionally with `github_token`) |
| `GET` | `/{username}` | View user profile + their projects |
| `GET` | `/{username}/auth/github` | Start GitHub OAuth flow (redirect to GitHub) |
| `GET` | `/auth/github/callback` | GitHub OAuth callback handler |
| `GET` | `/{username}/github/status` | Check GitHub connection status (JSON) |
| `GET` | `/{username}/github/repos` | List connected user's GitHub repos (JSON) |
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
