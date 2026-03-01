# Cache — Nix Binary Cache Server

## What

A nix-serve–compatible binary cache HTTP server. Serves `/{hash}.narinfo` and `/nar/{file}` endpoints, querying the local Nix store and optionally signing packages with an ed25519 key.

## Why

The cache is a shared acceleration layer for the entire pipeline, not a CI-only artifact store. The "push-first" model leverages Nix reproducibility:

1. **Users push first** — `pcr push` builds closures locally and pushes to the cache, doing work upfront
2. **CI pulls from cache** — when triggered by git push, CI checks the cache first. If closures are already there, CI skips the build entirely
3. **CI pushes on miss** — if the cache doesn't have the closure, CI builds and pushes
4. **Workers always pull** — workers fetch pre-built NARs from cache, never build locally

This means CI is usually fast (cache hit from the user's push) and workers are always fast (pre-built NARs).

## Endpoints

| Path | Purpose |
|------|--------|
| `/{hash}.narinfo` | NAR metadata (store path, hash, size, references, signature) |
| `/nar/{file}` | Compressed NAR archive content |
| `/nix-cache-info` | Cache metadata for Nix clients |
