# Plan: GitHub OAuth Connection and Repo Import

## Change Summary

Users currently must manually create and paste a GitHub Personal Access Token
(PAT) via `POST /{username}/github-token`. This plan replaces that flow with
a proper GitHub OAuth connection and adds the ability to browse and import
repositories from GitHub through the web UI.

The result: users click "Connect to GitHub" on their profile, authorize via
GitHub's OAuth page, and the token is stored automatically. They can then
browse their GitHub repos from the UI and import them into projects with a
single click.

## Success Criteria

1. A user can connect their GitHub account via OAuth from their profile page
2. The OAuth callback stores the token automatically in `users.github_token`
3. The user's profile page shows "Connected as {github_login}" with a disconnect option
4. A user with a connected GitHub can browse their repos from the UI
5. A user can import a GitHub repo (select → auto-fill git_url → create repo)
6. Manual repo creation (current flow) still works unchanged
7. OAuth setup is documented in README (env vars, GitHub App registration)

## Constraints and Non-Goals

- **No login/auth system** — repohub has no concept of "logged in user". The OAuth flow is identified by the username in the URL path (`/{username}/auth/github`). The `state` OAuth parameter carries the username for callback correlation.
- **No local clone** — imported repos store the GitHub `git_url` only (option B). The existing `create_or_clone_repository` flow is preserved for manually created repos that need local bare repos.
- **No webhook registration** — post-import webhook setup is out of scope.
- **No multi-page import wizard** — a single "Import from GitHub" selector on the repo creation page is sufficient for v1.
- **No token encryption** — the token is stored in plaintext in SQLite. Encryption is future work.

## Task Stack

- [x] T01: `Add GitHub OAuth config and authorization redirect endpoint` (status:done)
  - Task ID: T01
  - Goal: Add OAuth App config fields (`github_oauth_client_id`, `github_oauth_client_secret`, `github_oauth_redirect_url`) and implement `GET /{username}/auth/github` that redirects to GitHub's OAuth authorize URL with the username encoded in `state`.
  - Boundaries (in/out of scope): In - config fields, redirect endpoint, state encoding/validation. Out - callback handling (T02), UI button (T03).
  - Done when: Navigating to `/{username}/auth/github` redirects to `https://github.com/login/oauth/authorize?client_id=...&state=...&scope=repo`.
  - Verification notes: Check redirect URL matches expected GitHub OAuth format with correct client_id, scope, and state containing the username.
  - **Completed:** 2026-05-17
  - **Files changed:** `repohub/Cargo.toml`, `repohub/src/config.rs`, `repohub/src/adapters/github/web.rs`
  - **Evidence:** `cargo build -p repohub` succeeded; `cargo test -p repohub` 32/32 passed; `cargo fmt --check -p repohub` clean
  - **Notes:** Added `rand` and `base64` dependencies. Config fields default to empty strings. `GithubAppState` now stores `config: Config` and `oauth_nonces: Arc<Mutex<HashMap<String, String>>>`. The `GET /{username}/auth/github` handler generates a 16-byte hex nonce, stores it in the nonce map, builds `state = base64(username):nonce`, and redirects to GitHub's authorize endpoint with `scope=repo`.

- [x] T02: `Implement OAuth callback handler and token storage` (status:done)
  - Task ID: T02
  - Goal: Handle `GET /auth/github/callback?code=...&state=...` — exchange the code for an access token via GitHub API, decode the `state` to identify the user, verify the CSRF nonce, fetch the GitHub login from `/user`, and store both token and login in the users table. Add `github_login` column to the users table.
  - Boundaries (in/out of scope): In - callback handler at `GET /auth/github/callback`, token exchange with GitHub API, CSRF nonce verification + cleanup (10min TTL), DB migration for `github_login` column, update `UserRow` struct, store token + login in DB, fancy error Askama template (title + message + troubleshooting per error type: denied/invalid state/exchange failure/expired nonce), redirect to user profile on success. Out - UI indicators (T03), repo import (T04).
  - Done when: Completing OAuth flow stores token and GitHub login in DB. Nonce is consumed on success or expired after 10min. Errors render a template with troubleshooting tips. User is redirected to `/{username}` on success.
  - Verification notes: `cargo test -p repohub`; manual OAuth flow end-to-end (requires configured OAuth App).
  - **Completed:** 2026-05-17
  - **Files changed:** `repohub/src/adapters/shared/database.rs`, `repohub/src/adapters/github/dto.rs`, `repohub/src/adapters/github/web.rs`, `repohub/templates/oauth_error.html`, `context/repohub/github-oauth.md`
  - **Evidence:** `cargo build -p repohub` succeeded; `cargo test -p repohub` 32/32 passed; `cargo fmt --check -p repohub` clean
  - **Notes:** Nonce storage changed from `HashMap<String, String>` to `HashMap<String, (String, Instant)>` for TTL enforcement. Callback registered at `/auth/github/callback` (app root, not under username). Five distinct error templates with troubleshooting tips. DTOs added for GitHub token exchange and user info responses.

- [x] T03: `Add "Connected to GitHub" UI on user profile page` (status:done)
  - Task ID: T03
  - Goal: Update the `user.html` Askama template to show:
    - "Connect to GitHub" button (when no token stored)
    - "Connected as {github_login}" with "Disconnect" button (when token stored)
  - Add a `GET /{username}/github/status` endpoint and a `DELETE /{username}/github-token` endpoint for disconnect.
  - Boundaries (in/out of scope): In - profile page template changes, status endpoint, disconnect endpoint. Out - repo import UI (T04).
  - Done when: Profile page shows connection status. Disconnect clears the token. Connect button links to `/{username}/auth/github`.
  - Verification notes: `cargo test -p repohub`; manually check profile page before/after connecting.
  - **Completed:** 2026-05-23
  - **Files changed:** `repohub/src/domain/github.rs`, `repohub/src/adapters/github/web.rs`, `repohub/templates/user.html`, `repohub/templates/base.html`
  - **Evidence:** `cargo build -p repohub` succeeded; `cargo test -p repohub` 30/30 passed
  - **Notes:** Added `github_login` to `User` domain struct. Added `GET /{username}/github/status` (JSON status endpoint) and `DELETE /{username}/github-token` (disconnect handler) in web.rs. Updated `user.html` template with GitHub connection card showing "Connect to GitHub" button (links to `/{username}/auth/github`) or "Connected as {login}" with Disconnect button. Added `.btn-danger` CSS to `base.html`.

- [x] T04: `Add GitHub repo list API endpoint` (status:done)
  - Task ID: T04
  - Goal: Implement `GET /{username}/github/repos` that calls GitHub's `/user/repos` API using the stored token and returns a JSON list of the user's repositories (id, name, full_name, html_url, clone_url, private, description).
  - Boundaries (in/out of scope): In - endpoint, GitHub API call, error handling (no token, expired token, network error). Out - import UI (T05), filtering/organization.
  - Done when: Authenticated user can `GET /{username}/github/repos` and receive a JSON array of their GitHub repos.
  - Verification notes: `cargo test -p repohub`; `curl http://localhost:3001/me/github/repos` returns 200 with repo list when token set.
  - **Completed:** 2026-05-23
  - **Files changed:** `repohub/src/adapters/github/dto.rs`, `repohub/src/adapters/github/web.rs`
  - **Evidence:** `cargo build -p repohub` succeeded; `cargo test -p repohub` 30/30 passed
  - **Notes:** Added `GithubRepoItem` DTO (id, name, full_name, html_url, clone_url, private, description). Added `GET /{username}/github/repos` handler that fetches from `https://api.github.com/user/repos` using the stored OAuth Bearer token. Error handling covers: no token (400), user not found (404), network error (502), GitHub API error (502), parse error (502).

- [x] T05: `Add GitHub repo import to repository creation page` (status:done)
  - Task ID: T05
  - Goal: Add a "Import from GitHub" section to the project page or repository creation flow that:
    - Fetches repos from `GET /{username}/github/repos` (UI-side)
    - Displays a selectable list (repo name, visibility badge)
    - On selection, auto-fills `name` and `git_url` and creates the repository via existing `POST /{username}/{project}/repositories`
  - Boundaries (in/out of scope): In - UI selector, auto-create repo. Out - modifying existing repos, bulk import, webhooks.
  - Done when: User can select a GitHub repo from a list and have it created as a repository in the current project.
  - Verification notes: Manual E2E — connect GitHub → navigate to project → click Import → select repo → repo appears in project.
  - **Completed:** 2026-05-23
  - **Files changed:** `repohub/templates/project.html`
  - **Evidence:** `cargo build -p repohub` succeeded; `cargo test -p repohub` 30/30 passed
  - **Notes:** Added "Import from GitHub" as a third tab in the existing repository modal. JS loads connection status from `/{username}/github/status`, fetches repo list from `/{username}/github/repos`, renders selectable list with name + Public/Private badge, and imports via `POST /{username}/{project}/repositories`. Shows "Connect to GitHub first" link if user has no token.

- [x] T06: `Add OAuth setup documentation to README` (status:done)
  - Task ID: T06
  - Goal: Document how to register a GitHub OAuth App, the required redirect URL, and the env vars to configure.
  - Boundaries: README.md only.
  - Done when: README has a clear "GitHub OAuth Setup" section with step-by-step instructions and env var table.
  - Verification notes: Review rendered README.
  - **Completed:** 2026-05-23
  - **Files changed:** `repohub/README.md`
  - **Evidence:** `cargo build -p repohub` succeeded
  - **Notes:** Added "GitHub OAuth (Recommended)" section with 4-step setup guide (register OAuth App, configure env vars, run, connect in UI). Added env var table for OAuth config fields. Updated Available Endpoints table with all OAuth-related routes. OAuth fields added to Quick Start config table. Old PAT-based instructions retained as "Option B: Legacy".

- [x] T07: `Final validation and cleanup` (status:done)
  - Task ID: T07
  - Goal: Run full test suite, verify OAuth flow end-to-end (or document manual steps), update context files, remove scaffolding.
  - Done when: `cargo test -p repohub` passes. `cargo fmt --check` passes. Context reflects new GitHub OAuth and import capabilities.
  - Verification notes: `cargo test -p repohub`; `cargo fmt --all -- --check`.
  - **Completed:** 2026-05-23

## Validation Report

### Commands run
- `cargo fmt --all -- --check` → exit 0 (no formatting issues)
- `cargo test -p repohub` → exit 0 (32/32 tests passed: 25 unit + 5 main + 2 integration)
- Scaffolding check: no temporary files found in `context/tmp/`

### Success-criteria verification
- [x] **`cargo test -p repohub` passes** — 32/32 tests passed, 0 failed, 0 ignored
- [x] **`cargo fmt --all -- --check` passes** — clean exit, no formatting changes needed
- [x] **Context reflects new GitHub OAuth and import capabilities** — verified:
  - `context/plans/github-oauth-connection.md` — all tasks T01-T07 documented with evidence
  - `context/context-map.md` — current work updated through T07
  - `context/repohub/github-oauth.md` — complete documentation of OAuth flow, all endpoints, UI, import
  - `context/overview.md` — no changes needed (verify-only)
  - `context/architecture.md` — no changes needed (verify-only)
  - `context/glossary.md` — no changes needed (verify-only)

### Residual risks
- GitHub OAuth tokens are stored in plaintext in SQLite (no encryption). This is consistent with the pre-existing PAT storage and noted as future work.
- OAuth nonces are stored in-memory (`Arc<Mutex<HashMap>>`) and will be lost on server restart. Users in the middle of the OAuth flow will need to start over.
- No test coverage for the OAuth redirect, callback, or import UI (these require a running server with a configured OAuth App). Manual E2E steps are documented.

## Open Questions

None blocking. Resolved during T01 review:

1. **Config source**: Fields added to existing `Config` struct with empty-string defaults (matching existing pattern).
2. **State wiring**: Full `Config` stored in `GithubAppState`.
3. **CSRF protection**: `state` parameter includes a random nonce alongside the username; nonces stored in-memory via `Arc<Mutex<HashMap>>` on state.
4. **Redirect URL**: The full callback URL passed as `redirect_uri` in the OAuth authorization request.

## Assumptions

1. The operator registers a GitHub OAuth App at `https://github.com/settings/developers` with callback URL set to `http://host:port/auth/github/callback`.
2. The OAuth token does not expire (GitHub OAuth App tokens with `repo` scope are indefinitely valid unless revoked by the user).
3. The user's `github_token` column already exists in the database (added in previous work).
4. No login/auth system exists — the username in the URL path identifies the user for OAuth correlation.
