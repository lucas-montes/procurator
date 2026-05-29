# GitHub App Auth

This file documents the current GitHub App auth layer used by `repohub`.

## Current state

- `repohub/src/adapters/github/app_auth.rs` defines `GithubAppAuthenticator`.
  - Creates short-lived GitHub App JWTs with `RS256`.
  - Exchanges a JWT for an installation access token via `POST /app/installations/{installation_id}/access_tokens`.
  - Builds an authenticated `octocrab::Octocrab` client from the installation token.
- `repohub/src/adapters/github/auth.rs` defines `GithubAuth`.
  - Supports PAT-backed auth and GitHub App installation auth.
  - Caches installation tokens in-memory until expiry.
  - Exposes `get_token()` for low-level call sites and `octocrab_client()` for callers that want a ready-to-use API client.
  - Exposes `list_authenticated_user_repositories()` for the repo-import UI path, which now goes through the shared auth wrapper instead of raw HTTP calls.
- `repohub/src/adapters/github/web.rs` wires app auth into `GithubAppState` when `Config` contains a GitHub App ID and PEM contents.
  - The `GET /{username}/github/repos` route now calls the auth wrapper to fetch repositories.

## Usage pattern

1. Construct `GithubAppAuthenticator` from `Config` values.
2. Wrap it in `GithubAuth::from_app(...)` with an installation id.
3. Call `get_token().await` when a raw token is needed.
4. Call `octocrab_client().await` when a client is preferable.

## Notes

- The current implementation keeps the wrapper thin so auth details stay inside `app_auth.rs` and `auth.rs`.
- PAT support still exists for repository-owner token flows used by DORA refresh.
- The installation mapping question is still open at the plan level (`context/plans/github-app-migration.md`).

See also: [github-app-migration.md](../plans/github-app-migration.md), [github-oauth-testing.md](github-oauth-testing.md)
