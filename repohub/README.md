# Repohub — Project & Repository Management

## What

A web-based platform for managing users, projects, and Git repositories. Built with Axum, Askama templates, and SQLite. Renders a GitHub-inspired HTML interface and exposes a JSON API for creating entities.

## Why

Procurator needs a place where users create and organize the repositories that feed into the GitOps pipeline. When a repo is created here, it gets a bare Git repo on disk with a `post-receive` hook that triggers the CI service. Repohub is the entry point for all project configuration.

## Architecture

- **`web.rs`** — HTTP handlers + Askama HTML templates
- **`database.rs`** — SQLite persistence via sqlx
- **`models.rs`** — Domain entities (User, Project, Repository)
- **`config.rs`** — Application settings
- **`templates/`** — 7 HTML templates (index, user, project, repository, flake, configuration)

Uses `repo_outils` for Git operations and Nix flake metadata.

## Status

Scaffolded — CRUD for users/projects/repos is functional. Configuration management, Nix flake integration, build tracking, and E2E testing are planned.
