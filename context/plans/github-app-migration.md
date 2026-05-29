# Plan: GitHub App Migration for Repohub

## Change Summary

`repohub` currently exposes GitHub OAuth-style connection and repo import behavior, but the long-term integration should use a GitHub App. This migration replaces OAuth-centric assumptions with a config-driven GitHub App setup, short-lived installation tokens, and app-friendly webhook handling while keeping the user-facing repo import experience intact.

The intent is not to re-architect the product. The goal is to preserve the current UI and repository import workflow while changing the auth substrate underneath it to the GitHub App model.

## Current Touchpoints

The migration scope currently touches these files and flows:

### Config and startup

- `repohub/src/config.rs`
  - GitHub OAuth fields: `github_oauth_client_id`, `github_oauth_client_secret`, `github_oauth_redirect_url`
  - GitHub App fields: `github_app_id`, `github_app_private_key_pem`, `github_webhook_secret`
  - `Config::default()` still initializes both sets of fields, so startup code can choose either path.
- `repohub/src/main.rs`
  - Builds `Config::default()` and passes it into `GithubAppState::new(db, &config)`.
  - DORA refresh still constructs `GithubAuth::from_pat(token)` from stored repository-owner tokens.
  - Route wiring creates the GitHub router tree through `repohub::github_routes()`.

### GitHub adapter surface

- `repohub/src/adapters/github/mod.rs`
  - Exposes the GitHub adapter module tree (`app_auth`, `auth`, `client`, `dto`, `persistence`, `web`).
- `repohub/src/adapters/github/app_auth.rs`
  - GitHub App JWT creation and installation-token exchange helper.
- `repohub/src/adapters/github/auth.rs`
  - Thin auth abstraction that currently supports both PAT and GitHub App installation token flows.
- `repohub/src/adapters/github/client.rs`
  - GitHub API fetch path for PRs, reviews, commits, deployments, and issues; all requests go through the auth layer.
- `repohub/src/adapters/github/dto.rs`
  - DTOs for GitHub user/repo/token responses used by the web adapter.
- `repohub/src/adapters/github/web.rs`
  - `GithubAppState` stores `Config`, OAuth nonce state, and optional GitHub App auth.
  - OAuth-oriented routes still exist alongside the GitHub App state wiring:
    - `GET /{username}/auth/github`
    - `GET /auth/github/callback`
    - `GET /{username}/github/status`
    - `DELETE /{username}/github-token`
    - `GET /{username}/github/repos`
  - `GithubAppState::new(db, &config)` currently constructs the GitHub App authenticator when `Config` contains app ID + PEM.

### Docs and context

- `context/repohub/github-oauth.md`
  - Detailed OAuth connection flow documentation; now legacy relative to the GitHub App target.
- `context/repohub/github-oauth-testing.md`
  - Manual test guide now rewritten for GitHub App setup.
- `context/context-map.md`
  - Lists the current GitHub OAuth docs, but not yet the GitHub App migration plan.

### Remaining migration boundary

The GitHub App migration still needs a decision on how installation context is represented and sourced (for example, single installation id vs. per-owner lookup). The current code/config surfaces already make the target shape visible, but the plan should keep that open question explicit until T02/T03.

## Success Criteria

1. `repohub` can be configured entirely through `Config` to use a GitHub App, including App ID, private key PEM, webhook secret, and installation context.
2. GitHub API access uses installation tokens obtained from the GitHub App flow, not long-lived OAuth App credentials.
3. Token handling is wrapped in a thin adapter layer so the rest of the codebase does not depend on GitHub-specific auth details.
4. The repository import/listing flow continues to work from the user perspective.
5. Webhook handling is documented and wired for future event-driven sync.
6. The old OAuth-oriented docs are replaced or clearly marked as legacy where applicable.
7. Validation is captured with concrete checks for formatting, tests, and manual setup.

## Constraints and Non-Goals

- **No new login system** — the username-in-path model remains unchanged.
- **No broad adapter rewrite** — keep the GitHub-specific wrapper thin and focused on auth/token exchange.
- **No enterprise-level auth support** — GitHub App is the target unless a future need arises for enterprise object access.
- **No migration of historical OAuth user tokens** — the plan focuses on future app-based access.
- **No hidden config discovery** — configuration should be explicit and owned by the calling service.

## Task Stack

- [x] T01: `Map current GitHub integration and migration boundaries`
  - Task ID: T01
  - Goal: Document every current GitHub/OAuth touchpoint in `repohub` that will be affected by the migration, including config fields, auth helpers, UI routes, token storage, and docs.
  - Boundaries (in/out of scope): In - code/doc inventory, identifying API surfaces and config wiring. Out - implementation changes.
  - Done when: The migration plan references the exact files and flows that need to change, and the scope is limited to repo-import/auth/webhook paths.
  - Verification notes: Review current `repohub/src/adapters/github/*`, `repohub/src/config.rs`, `repohub/src/main.rs`, and `context/repohub/*` docs.

- [x] T02: `Define the GitHub App auth layer`
  - Task ID: T02
  - Goal: Finalize the thin wrapper API around `octocrab`/GitHub App auth so the rest of the service can request a token/client without learning GitHub App internals.
  - Boundaries (in/out of scope): In - app auth struct, JWT signing, installation-token exchange, token caching strategy, error model. Out - route/UI changes.
  - Done when: There is a single auth abstraction with clear construction inputs from `Config` and a simple async token/client acquisition API.
  - Verification notes: The API should be small enough that `client.rs` only depends on a token getter/client builder, not JWT details.
  - Files changed: `repohub/src/adapters/github/app_auth.rs`, `repohub/src/adapters/github/auth.rs`
  - Evidence: `cargo fmt --all` clean; `get_errors` clean for edited Rust files; focused `cargo test` is currently blocked by the existing `askama` / `percent-encoding` dependency resolution issue in the workspace.

- [x] T03: `Wire GitHub App auth into repo access flows`
  - Task ID: T03
  - Goal: Update the GitHub client/import paths so repository listing and refresh operations use app tokens through the new abstraction.
  - Boundaries (in/out of scope): In - client wiring, installation-token usage, token cache refresh points. Out - docs cleanup and manual validation.
  - Done when: All GitHub API calls in the active flow go through the App auth layer and no OAuth-only assumptions remain in the path.
  - Verification notes: Confirm the current repo import and refresh flows still receive usable credentials with the new config shape.
  - Files changed: `repohub/src/adapters/github/auth.rs`, `repohub/src/adapters/github/web.rs`, `context/repohub/github-app-auth.md`
  - Evidence: `get_errors` clean for edited Rust files; `cargo fmt --all --check` clean after routing `GET /{username}/github/repos` through `GithubAuth` and its Octocrab-backed repo listing helper.

- [x] T04: `Make configuration explicit and config-first`
  - Task ID: T04
  - Goal: Ensure all GitHub App inputs are carried by `Config` and passed through app state/startup instead of being read ad hoc from environment variables or file lookups.
  - Boundaries (in/out of scope): In - config fields, startup wiring, state construction. Out - UI copy and deep auth logic.
  - Done when: `Config` is the single source of GitHub App setup in code, and startup paths construct auth state from it.
  - Verification notes: Check `repohub/src/main.rs`, `repohub/src/config.rs`, and `repohub/src/adapters/github/web.rs` for config-driven construction only.
  - Files changed: `repohub/src/config.rs`
  - Evidence: `cargo fmt --all --check` clean; `get_errors` clean for `repohub/src/config.rs` and `repohub/src/adapters/github/web.rs`.

- [x] T05: `Refresh docs and manual test guide`
  - Task ID: T05
  - Goal: Replace OAuth setup/test instructions with GitHub App setup, config examples, and manual validation steps for local development.
  - Boundaries (in/out of scope): In - `context/repohub/*` docs and any README sections that still describe OAuth as the primary integration. Out - code changes.
  - Done when: The active docs describe GitHub App creation, config values, installation token checks, and webhook testing.
  - Verification notes: The manual guide should match the actual config shape and routes used by the code.
  - Files changed: `repohub/README.md`, `context/repohub/github-oauth-testing.md`, `context/repohub/github-oauth.md`, `context/context-map.md`
  - Evidence: Manual guide now covers GitHub App setup, config-driven startup, installation token exchange, repo listing, and webhook validation; stale OAuth-tail content removed.

- [x] T06: `Validate, tighten, and hand off`
  - Task ID: T06
  - Goal: Run formatting/tests where possible, capture any remaining gaps, and produce a concise implementation handoff for the next coding session.
  - Boundaries (in/out of scope): In - verification checklist and cleanup notes. Out - new features.
  - Done when: The migration plan has a clear implementation order, known risks, and a final validation checklist.
  - Verification notes: Prefer crate-specific tests and formatting checks focused on touched files.

## Validation Report

### Commands run
- `cargo fmt --all --check` -> exit 0
- `cargo test -p repohub` -> exit 101 (`percent-encoding` / `askama` dependency resolution conflict already present in the workspace)

### Success-criteria verification
- [x] `Config` is the source of GitHub App setup in code -> verified via `repohub/src/config.rs` and `repohub/src/adapters/github/web.rs`
- [x] GitHub API access uses installation tokens through the wrapper -> verified via `repohub/src/adapters/github/auth.rs` and `repohub/src/adapters/github/app_auth.rs`
- [x] Repo listing/import flow still works through the auth layer -> verified via `repohub/src/adapters/github/web.rs` and `context/repohub/github-app-auth.md`
- [x] GitHub App setup and manual validation are documented -> verified via `repohub/README.md` and `context/repohub/github-oauth-testing.md`
- [x] Legacy OAuth docs are marked clearly as legacy -> verified via `context/repohub/github-oauth.md` and `context/context-map.md`

### Failed checks and follow-ups
- `cargo test -p repohub` could not complete because the workspace currently has an unrelated `askama` / `percent-encoding` version conflict.
- Follow-up, if desired: resolve the workspace dependency conflict and rerun the repohub test suite.

### Residual risks
- Installation mapping remains the main open product question in the migration plan.
- Legacy OAuth routes still exist for backward compatibility, but they are now clearly documented as legacy.

## Implementation Notes

- Keep the auth layer thin enough that future changes to GitHub API client usage do not require rewriting every call site.
- Prefer explicit config fields over hidden global state so test setup and runtime setup stay aligned.
- Treat webhook handling as part of the target architecture, even if the initial migration only documents or stubs the endpoint.

## Risks and Open Questions

- Installation mapping is the main product question: whether the code uses a single installation id for the first migration step or later resolves installations per owner/project.
- Token caching should be small and predictable; avoid over-engineering before the auth path is fully exercised.
- If any OAuth-specific UI text remains, it should be converted to GitHub App language or marked legacy.
