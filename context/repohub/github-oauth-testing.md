# GitHub OAuth — Manual Test Guide

How to manually test the GitHub OAuth connection and repo import flow end-to-end.

## Prerequisites

1. **A GitHub OAuth App** — register one at https://github.com/settings/developers → **New OAuth App**:
   - Homepage URL: `http://localhost:3001`
   - Callback URL: `http://localhost:3001/auth/github/callback`
   - Copy the **Client ID** and generate a **Client Secret**

2. **Start repohub** with OAuth enabled:

```bash
cd /home/lucas/Projects/procurator

GITHUB_OAUTH_CLIENT_ID="your_client_id" \
GITHUB_OAUTH_CLIENT_SECRET="your_client_secret" \
GITHUB_OAUTH_REDIRECT_URL="http://localhost:3001/auth/github/callback" \
cargo run -p repohub
```

You should see `Listening on 0.0.0.0:3001`.

## Test 1: Create a user

```bash
curl -X POST http://localhost:3001/users \
  -H "Content-Type: application/json" \
  -d '{"username": "testuser"}'
```

Expected: `200 OK`. Visit `http://localhost:3001/testuser` — profile page shows **"Connect to GitHub"** button.

## Test 2: Connect via OAuth

1. Open `http://localhost:3001/testuser` in a browser
2. Click **"Connect to GitHub"** — redirected to GitHub's authorization page
3. Authorize the app
4. Redirected back to `http://localhost:3001/testuser`
5. **Expected:** Profile shows **"Connected as *your_github_handle*"** with a **Disconnect** button

Check JSON status:

```bash
curl http://localhost:3001/testuser/github/status
```

Expected:
```json
{"connected": true, "github_login": "your_github_handle"}
```

## Test 3: List GitHub repos

```bash
curl http://localhost:3001/testuser/github/repos
```

Expected: JSON array of repos with `id`, `name`, `full_name`, `html_url`, `clone_url`, `private`, `description`.

## Test 4: Disconnect

1. On profile page, click **Disconnect**
2. Confirm in the dialog
3. **Expected:** Page reloads, shows **"Connect to GitHub"** button again

Or via curl:

```bash
curl -X DELETE http://localhost:3001/testuser/github-token
```
Expected: `200 OK` — "GitHub disconnected".

Re-connect before Test 5.

## Test 5: Import a GitHub repo

Create a project:

```bash
curl -X POST http://localhost:3001/testuser/projects \
  -H "Content-Type: application/json" \
  -d '{"name": "my-project"}'
```

1. Open `http://localhost:3001/testuser/my-project` in a browser
2. Click **"New Repository"**
3. In the modal, click **"Import from GitHub"** tab
4. **Expected:** List of GitHub repos appears with Public/Private badges
5. Click a repo to select it, then click **"Import Selected Repository"**
6. **Expected:** Page reloads, repo shows in the list

## Test 6: PAT flow still works (backward compat)

```bash
curl -X POST http://localhost:3001/testuser/github-token \
  -H "Content-Type: application/json" \
  -d '{"token": "ghp_your_pat_here"}'
```

Expected: `200 OK`.

## Test 7: Error cases

| What to test | How | Expected |
|---|---|---|
| OAuth denied | Click "Connect" then cancel on GitHub | "Access Denied" error template |
| Not connected | Open project page without connecting | "Import from GitHub" tab shows "Connect your GitHub account first" link |
| Empty account | Connected to GitHub account with no repos | "No repositories found" in import tab |
| Invalid username | `curl http://localhost:3001/nonexistent/github/repos` | `404 Not Found` |

## Quick smoke test (no browser)

```bash
# Start with OAuth
GITHUB_OAUTH_CLIENT_ID="x" GITHUB_OAUTH_CLIENT_SECRET="x" GITHUB_OAUTH_REDIRECT_URL="http://localhost:3001/auth/github/callback" cargo run -p repohub &

# In another terminal:
curl -s http://localhost:3001/testuser/github/status           # {"connected":false}
curl -s http://localhost:3001/testuser/github/repos             # 400 - no token
curl -s -X DELETE http://localhost:3001/testuser/github-token   # 200 - disconnected (no-op)
```

See also: [github-oauth.md](github-oauth.md) — feature documentation, [plan](../plans/github-oauth-connection.md)
