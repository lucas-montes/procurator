# Autonix — Automatic Nix Flake Generator

## What

A library that scans repositories, detects their tech stack, and auto-generates Nix flake files. Analyzes project structure by inspecting config files (Dockerfiles, lockfiles, CI configs, task runners) and language markers, then renders a `flake.nix` using Jinja templates.

## Why

The goal is twofold:

1. **Package the app** — scan the repo, detect the language/framework, and generate a flake that builds it into a Nix derivation. Users shouldn't have to write Nix by hand to get started.
2. **Discover dependencies** — detect databases, proxies, message queues, and other infrastructure the app needs, then use that information to generate the cluster topology.

Used directly by the **CLI** (`pcr init`). Intended to also be used by **Repohub** when a project is created, but that integration doesn't exist yet. It lowers the barrier from "learn Nix" to "push your code."

## Modules

- **`mapping/`** — Detection rules: languages, lockfiles, manifests, containers, CI files, task runners, version files
- **`repo/`** — Repository scanning, analysis, and flake generation
- **`project/`** — Project-level parsing (multi-repo); not implemented yet, see Status
- **`templates/`** — Jinja templates for flake output (`flake.jinja`)

## Status

Both halves of the stated goal are implemented for a single repo: `repo/scan.rs` → `repo/analysis.rs` extracts packages/toolchains/checks (goal 1) and `extract_services` detects infrastructure dependencies such as databases and queues from CI service definitions and docker-compose images (goal 2); `repo/flake.rs` renders the result into a real `flake.nix` via `flake.jinja`, with test fixtures for Rust and JS/Python monorepos.

Missing to reach the full goal:

- **Multi-repo project parsing is not implemented.** `project/mod.rs` is a 4-line design note describing the intent (grouping multiple repos' services so they can be launched together with their shared dependencies); `project/parser.rs` is an empty file. Single-repo analysis works; nothing generates a project-level flake across repos yet.
- **Repohub integration doesn't exist.** `cli`'s `pcr init` calls into `autonix::Parser`, but `repohub` has no reference to this crate at all, despite being a stated consumer.

## Origin

Based on [davidabram/autonix](https://github.com/davidabram/autonix). Test fixtures are adapted from that project.
