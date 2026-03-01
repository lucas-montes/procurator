# Repohub — Project & Repository Management

## What

A library (with optional web binary) for managing users, projects, and Git repositories. Built with Axum, Askama templates, and SQLite. Renders a GitHub-inspired HTML interface and exposes a JSON API.

## Why

Procurator needs a place where users create and organize the repositories that feed into the GitOps pipeline. When a repo is created, repohub sets up a bare Git repo on disk with a `post-receive` hook that triggers the CI service. It’s the entry point for all project configuration.

## Library-First Design

Structured as a library (`repohub::*`) with a thin binary (`main.rs`). The library can be embedded directly into a monolith alongside ci_service, or run as a standalone web server. When deployed together, repohub can call ci_service functions directly; when separate, the `post-receive` hook calls CI’s HTTP API.

## Architecture

- **`web.rs`** — HTTP handlers + Askama HTML templates
- **`database.rs`** — SQLite persistence via sqlx
- **`models.rs`** — Domain entities (User, Project, Repository)
- **`config.rs`** — Application settings
- **`templates/`** — 7 HTML templates (index, user, project, repository, flake, configuration)

Uses `repo_outils` for Git operations and Nix flake metadata.

## Status

Scaffolded — CRUD for users/projects/repos is functional. Configuration management, Nix flake integration, build tracking, and E2E testing are planned.
