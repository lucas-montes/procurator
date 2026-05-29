# GitHub App — Manual Test Guide

This guide covers the GitHub App auth path used by `repohub`.

Overview
- For server-side automation and webhook handling, `repohub` should use a GitHub App (not an OAuth App). GitHub Apps provide fine-grained permissions, installation-scoped tokens (short-lived), and built-in webhooks.

## Prerequisites — create a GitHub App

1. Go to https://github.com/settings/apps and choose **New GitHub App** (or the organization settings to create an org-owned app).
   - **Application name:** `repohub` (or your choice)
   - **Homepage URL:** `http://localhost:3001`
   - **Webhook URL:** `http://localhost:3001/github/webhook`
   - **Webhook secret:** store this locally as `GITHUB_WEBHOOK_SECRET`
   - **Permissions:** grant only what's needed; typical choices:
     - Repositories: `Contents` (read), `Contents` (read & write) only if importing/git operations are required
     - Pull requests: read
     - Issues: read
     - Metadata: read
     - (CI/status) `Commit statuses`: write if you need to report statuses
   - **Subscribe to events:** `push`, `pull_request`, `repository`, `installation` (at minimum `installation`/`installation_repositories` to track installs)
   - After creating, **Generate a private key** and download the PEM file. Note the **App ID** and save the private key.

2. (Optional) Install the app on your user/org and one or more repositories for local testing. When installed, note the **Installation ID** (you can see it in the URL of the installation or via the API).

## Config-driven setup

Populate the `repohub::Config` object before constructing `GithubAppState`:

```rust
use repohub::Config;

let config = Config {
  github_app_id: Some(12345),
  github_app_private_key_pem: Some(include_str!("/path/to/github_app.pem").to_string()),
  github_webhook_secret: Some("your_webhook_secret".to_string()),
  ..Config::default()
};

```

Once `GithubAppState::new(db, &config)` is called, `repohub` can generate short-lived installation access tokens and call the GitHub API to list repositories, fetch metadata, and receive webhooks.

## Test 1: Service starts and reads config

Start the service with the config populated above. Expected: the service logs should show that it loaded the GitHub App config without printing secrets.

## Test 2: Installation token exchange (manual check)

If you have an installation id you can exercise the token exchange.

```bash
# (this is what the repohub helper performs)
curl -X POST \
  -H "Authorization: Bearer <JWT>" \
  -H "Accept: application/vnd.github+json" \
  https://api.github.com/app/installations/<INSTALLATION_ID>/access_tokens
```

Expected: JSON with `token` and `expires_at`. The token is valid for ~1 hour and is used as the `Authorization: token <token>` header for subsequent API calls.

## Test 3: Import flow and repo listing

With a valid installation token the app should be able to list repositories the installation has access to and populate the import modal. Exercise the normal UI flow:

1. Create a user in `repohub` (same as before):

```bash
curl -X POST http://localhost:3001/users \
  -H "Content-Type: application/json" \
  -d '{"username": "testuser"}'
```

2. Visit that user page and use the **Import from GitHub** flow — the repo list should show repos the App installation can access (public/private as configured).

## Test 4: Webhooks

1. Configure your local environment to receive webhooks at `http://localhost:3001/github/webhook` (use `ngrok` or similar if GitHub needs a public URL).
2. Trigger a repository event (e.g., open a PR or push) and verify `repohub` receives and processes the webhook. The service should validate the webhook signature against `GITHUB_WEBHOOK_SECRET`.

## Test 5: Backward compatibility with PATs (if present)

The repository still supports storing a personal access token for user-scoped operations. That flow should continue to work for user-facing features, but server automation should prefer the GitHub App installation tokens.

## Troubleshooting

- If you get `403` when fetching repositories, check the App permissions and the repositories selected during installation.
- If webhooks aren't arriving, ensure the webhook URL is reachable and the secret matches the value in `config.github_webhook_secret`.
- Use the installation listing API (`GET /app/installations`) with a JWT to discover installation IDs.

See also: `context/repohub/github-oauth-testing.md` (now GitHub App guide), `repohub` source `repohub/src/adapters/github/app_auth.rs` for the implementation helper.
