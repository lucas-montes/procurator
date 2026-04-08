# Repohub — Project & Repository Management

## What

A library (with optional web binary) for managing users, projects, and Git repositories. Built with Axum, Askama templates, and SQLite. Renders a GitHub-inspired HTML interface and exposes a JSON API.

## Why

Procurator needs a place where users create and organize the repositories that feed into the GitOps pipeline. When a repo is created, repohub sets up a bare Git repo on disk with a `post-receive` hook that triggers the CI service. It’s the entry point for all project configuration.

## Library-First Design

Structured as a library (`repohub::*`) with a thin binary (`main.rs`). The library can be embedded directly into a monolith alongside ci_service, or run as a standalone web server. When deployed together, repohub can call ci_service functions directly; when separate, the `post-receive` hook calls CI’s HTTP API.

## Architecture

- **`adapters/github/web.rs`** — legacy GitHub-like HTTP handlers + Askama templates
- **`adapters/gerrit/web.rs`** — Gerrit HTTP handlers (API + UI)
- **`adapters/shared/database.rs`** — SQLite persistence via sqlx
- **`config.rs`** — Application settings
- **`domain/`** — core entities and value objects
- **`application/`** — ports and use-cases
- **`templates/`** — HTML templates for GitHub-like and Gerrit screens

### Hexagonal Modules (Gerrit Review System)

- **`domain/`** — review aggregates and Gerrit-default policy rules
- **`application/`** — use-cases + ports (boxed-future async boundaries)
- **`adapters/gerrit/persistence.rs`** — SQLite adapter implementing review ports
- **`adapters/gerrit/web.rs`** — inbound HTTP adapter (UI + JSON review routes)

The crate keeps existing project/repository flows while layering Gerrit-style review capabilities behind ports/adapters. In `main.rs`, Gerrit routes are mounted under `/gerrit`.

## Gerrit Guide

Detailed Gerrit behavior (change/patchset logic, votes/readiness, submit flow, and API/UI examples) is documented in:

- **`docs/gerrit.md`**

Quick summary:

- Gerrit routes are mounted under `/gerrit`
- API and UI share the same underlying port-backed logic
- UI differs only in final rendering (Askama templates), API returns JSON

## Gerrit-Style Review Endpoints

Base path is mounted under `/gerrit` and uses repository context:

- `POST /gerrit/{username}/{project}/{repo}/changes` — create change
- `GET /gerrit/{username}/{project}/{repo}/changes` — list changes (JSON)
- `GET /gerrit/{username}/{project}/{repo}/changes/ui` — list changes (HTML)
- `GET /gerrit/{username}/{project}/{repo}/changes/{change_id}/ui` — change detail (HTML)
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/patchsets` — upload patch set
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/votes` — add/update label vote
- `GET /gerrit/{username}/{project}/{repo}/changes/{change_id}/submit-readiness` — evaluate submit checks
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/submit` — submit change (marks `Merged` when checks pass)
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/abandon` — abandon change
- `POST /gerrit/{username}/{project}/{repo}/changes/{change_id}/restore` — restore abandoned change to `New`

### Where the Gerrit UI is in the app

- Open any repository page (GitHub-like UI)
- Click **"Open Gerrit Changes"**
- That takes you to `/gerrit/{username}/{project}/{repo}/changes/ui`
- Click any change subject to open detail/actions page

### Example payloads

Create change:

```json
{
	"subject": "Add scheduler metrics",
	"target_branch": "master",
	"revision": "f00ba4123",
	"kind": "ref_upload"
}
```

Vote:

```json
{
	"reviewer_username": "alice",
	"label": "Code-Review",
	"value": 2
}
```

Default Gerrit policy:

- Labels: `Code-Review (-2..+2)`, `Verified (-1..+1)`
- Required for readiness: `Code-Review >= +2`, `Verified >= +1`
- Submit type default: `RebaseIfNecessary`

Uses `repo_outils` for Git operations and Nix flake metadata.

## Status

Scaffolded — CRUD for users/projects/repos is functional. Configuration management, Nix flake integration, build tracking, and E2E testing are planned.
