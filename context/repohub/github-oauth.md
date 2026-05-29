# GitHub OAuth Connection (Legacy)

Legacy reference for the older OAuth-based GitHub account connection flow.
The current server-side GitHub integration is documented in
`context/repohub/github-app-auth.md` and the GitHub App manual test guide.

This doc is kept for historical context and for deployments that still use the
username-based OAuth/PAT account flow.

## Config Fields

Added to `repohub/src/config.rs` (`Config` struct):

| Field | Type | Default | Purpose |
|---|---|---|---|
| `github_oauth_client_id` | `String` | `""` | GitHub OAuth App client ID |
| `github_oauth_client_secret` | `String` | `""` | GitHub OAuth App client secret |
| `github_oauth_redirect_url` | `String` | `""` | Full callback URL (e.g. `http://host:3001/auth/github/callback`) |

Empty-string defaults mean the feature is disabled by default.

## State Wiring

`GithubAppState` in `repohub/src/adapters/github/web.rs`:

- `config: Config` — full config stored for OAuth fields
- `oauth_nonces: Arc<Mutex<HashMap<String, (String, Instant)>>>` — in-memory nonce store: `nonce → (username, created_at)`

`GithubAppState::new(db, config)` initializes both fields (config cloned, nonces map empty).

## Authorization Redirect

**Route:** `GET /{username}/auth/github`

**Flow:**
1. Check `client_id` and `redirect_uri` are non-empty; return 500 if unconfigured
2. Generate 16 random bytes → hex-encoded nonce string
3. Store `nonce → (username, Instant::now())` in `oauth_nonces` map
4. Build `state = base64(username):nonce`
5. Redirect (302) to `https://github.com/login/oauth/authorize?client_id=...&redirect_uri=...&state=...&scope=repo`

## Callback Handler

**Route:** `GET /auth/github/callback?code=...&state=...`

Registered at app root (not under username prefix).

**Flow:**
1. Check for `?error=access_denied` → render "Access Denied" error template
2. Parse `code` and `state` query params
3. Decode state: `base64(username):nonce`
4. Verify nonce in `oauth_nonces` map, check 10-minute TTL (`Duration::from_secs(600)`)
5. Remove consumed nonce from map
6. POST to `https://github.com/login/oauth/access_token` with `client_id`, `client_secret`, `code`, `redirect_uri`
7. Parse `access_token` from JSON response; on error → render "GitHub Error" template
8. GET `https://api.github.com/user` with `Authorization: Bearer {token}`
9. Extract `login` from response
10. Look up user by username in DB
11. Store token via `update_user_github_token`, store login via `update_user_github_login`
12. Redirect (302) to `/{username}` on success

**Error templates rendered per failure type:**

| Error | Template Title | Example Tip |
|---|---|---|
| User denied on GitHub | "Access Denied" | Click the Connect button to try again |
| Invalid state format / nonce missing | "Invalid Request" | Start the OAuth flow again |
| Nonce expired (>10 min) | "Request Expired" | Complete authorization within 10 minutes |
| Token exchange failure | "GitHub Error" | Verify OAuth App configuration |
| User not found in DB | "Invalid Request" | Create a user account first |

Template: `templates/oauth_error.html` — extends `base.html`, shows error title, message, troubleshooting tips list, and a "Go Back" link.

## CSRF Protection

The `state` parameter contains:
- `base64(username)` — identifies the user on callback
- `:nonce` — random hex nonce, stored in-memory at `oauth_nonces`

The callback verifies the nonce exists, checks the 10-minute TTL (`Instant::now() - created_at > 600s`), removes it atomically, and rejects expired or mismatched nonces.

## Database

**Column added to `users` table:** `github_login TEXT`

`UserRow` struct updated to include `pub github_login: Option<String>`.

Methods:
- `update_user_github_login(user_id, github_login)` — UPDATE users SET github_login = ?

All `SELECT` queries on `users` now include `github_login`. Migration via `ALTER TABLE users ADD COLUMN github_login TEXT` in `initialize_tables()`.

## Dependencies

- `rand = "0.8"` — random nonce generation
- `base64 = "0.21"` — username encoding in state parameter
- `reqwest` (already present) — GitHub API calls for token exchange and user info

## Profile UI

**Route (template):** `GET /{username}` renders `templates/user.html`

The profile page now includes a GitHub connection card:

- **Not connected** (no `github_token`): Shows a "Connect to GitHub" button linking to `GET /{username}/auth/github`
- **Connected** (token stored): Shows "Connected as **{github_login}**" with a "Disconnect" button

The `User` domain struct includes `github_login: Option<String>` which the template uses to determine which state to render.

## Status Endpoint

**Route:** `GET /{username}/github/status`

Returns JSON indicating the user's GitHub connection state:

```json
// Connected:
{ "connected": true, "github_login": "octocat" }

// Not connected:
{ "connected": false }
```

## Disconnect Endpoint

**Route:** `DELETE /{username}/github-token`

Clears both `github_token` and `github_login` from the user's database row. Returns `200 OK` on success. Shows a confirmation prompt on the client side before sending the request.

## Repo List Endpoint

**Route:** `GET /{username}/github/repos`

Fetches the user's GitHub repositories by calling `GET https://api.github.com/user/repos?per_page=100&sort=updated&type=all` using the stored OAuth token (sent as `Authorization: Bearer {token}`).

**Response:** JSON array of repo items:

```json
[
  {
    "id": 123456789,
    "name": "my-repo",
    "full_name": "octocat/my-repo",
    "html_url": "https://github.com/octocat/my-repo",
    "clone_url": "https://github.com/octocat/my-repo.git",
    "private": false,
    "description": "A sample repository"
  }
]
```

**Error responses:**
- `404` — User not found
- `400` — No GitHub token configured (user hasn't connected via OAuth)
- `502` — GitHub API unreachable, returned an error, or returned unparseable data

The DTO `GithubRepoItem` (in `repohub/src/adapters/github/dto.rs`) deserializes and serializes the exact subset of fields listed above. Tokens are sent using `Bearer` auth (matching the OAuth token format from GitHub's access token response).

## Import from GitHub (UI)

The project page (`templates/project.html`) now includes an "Import from GitHub" tab in the repository creation modal.

**Flow:**
1. User opens the repo modal and clicks "Import from GitHub" tab
2. JS checks `GET /{username}/github/status`:
   - If **not connected**: Shows "Connect your GitHub account first" with a link to the user's profile (`/{username}`)
   - If **connected**: Fetches `GET /{username}/github/repos` and renders a selectable list
3. Each repo shown with `full_name`, description (if any), and a visibility badge (green "Public" / gold "Private")
4. User clicks a repo to select it, then clicks "Import Selected Repository"
5. JS calls `POST /{username}/{project}/repositories` with `{ name, git_url: clone_url }` from the selected repo
6. On success, page reloads showing the imported repo

No server-side changes were needed — all APIs already existed from T01-T04.

## Key Files

| Path | Role |
|---|---|
| `repohub/src/config.rs` | OAuth config fields |
| `repohub/src/adapters/github/web.rs` | `GithubAppState`, auth redirect + callback handlers, OAuthErrorTemplate, route registration |
| `repohub/src/adapters/github/dto.rs` | `GithubAccessTokenResponse`, `GithubUserResponse`, `GithubRepoItem` DTOs |
| `repohub/src/adapters/shared/database.rs` | `UserRow` with `github_login`, `update_user_github_login`, migration |
| `repohub/templates/oauth_error.html` | Error template with title, message, troubleshooting tips |
| `repohub/templates/user.html` | Profile template with GitHub connection card |
| `repohub/templates/base.html` | `.btn-danger` CSS for disconnect button |
| `repohub/templates/project.html` | Project page with "Import from GitHub" modal tab |
| `repohub/src/domain/github.rs` | `User` struct with `github_login` field |
| `repohub/Cargo.toml` | `rand` and `base64` dependencies |

See also: [repohub README](../../repohub/README.md), [plan (completed)](../plans/github-oauth-connection.md)
