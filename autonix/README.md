# Autonix — Automatic Nix Flake Generator

## What

A library that scans repositories, detects their tech stack, and auto-generates Nix flake files. Analyzes project structure by inspecting config files (Dockerfiles, lockfiles, CI configs, task runners) and language markers, then renders a `flake.nix` using Jinja templates.

## Why

The goal is twofold:

1. **Package the app** — scan the repo, detect the language/framework, and generate a flake that builds it into a Nix derivation. Users shouldn't have to write Nix by hand to get started.
2. **Discover dependencies** — detect databases, proxies, message queues, and other infrastructure the app needs, then use that information to generate the cluster topology.

Used directly by the **CLI** (`pcr init`) and **Repohub** (when a project is created). It lowers the barrier from "learn Nix" to "push your code."

## Modules

- **`mapping/`** — Detection rules: languages, lockfiles, manifests, containers, CI files, task runners, version files
- **`repo/`** — Repository scanning, analysis, and flake generation
- **`project/`** — Project-level parsing (multi-repo)
- **`templates/`** — Jinja templates for flake output (`flake.jinja`)

## Origin

Based on [davidabram/autonix](https://github.com/davidabram/autonix). Test fixtures are adapted from that project.
