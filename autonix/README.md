# Autonix — Automatic Nix Flake Generator

## What

A library that scans repositories, detects their tech stack, and auto-generates Nix flake files. Analyzes project structure by inspecting config files (Dockerfiles, lockfiles, CI configs, task runners) and language markers, then renders a `flake.nix` using Jinja templates.

## Why

When users onboard a new project into procurator, they shouldn't have to write a Nix flake by hand. Autonix inspects the repo and produces a reasonable starting flake — lowering the barrier from "learn Nix" to "push your code." Called by `repohub` and `ci_service` during project setup.

## Modules

- **`mapping/`** — Detection rules: languages, lockfiles, manifests, containers, CI files, task runners, version files
- **`repo/`** — Repository scanning, analysis, and flake generation
- **`project/`** — Project-level parsing (multi-repo)
- **`templates/`** — Jinja templates for flake output (`flake.jinja`)

## Origin

Based on [davidabram/autonix](https://github.com/davidabram/autonix). Test fixtures are adapted from that project.
