# Cache — Nix Binary Cache Server

## What

A nix-serve–compatible binary cache HTTP server. Serves `/{hash}.narinfo` and `/nar/{file}` endpoints, querying the local Nix store and optionally signing packages with an ed25519 key.

## Why

In the GitOps pipeline, CI builds Nix closures once and publishes them to the cache. Workers then pull pre-built NARs instead of rebuilding locally. This crate is the binary distribution layer that makes deployment fast — workers fetch hundreds of megabytes of pre-built NixOS systems rather than compiling from source.

## Endpoints

| Path | Purpose |
|------|---------|
| `/{hash}.narinfo` | NAR metadata (store path, hash, size, references, signature) |
| `/nar/{file}` | Compressed NAR archive content |
| `/nix-cache-info` | Cache metadata for Nix clients |
