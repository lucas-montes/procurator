# GitOps Workflow Reference

How a git push becomes a running VM. These are **reference steps** — the Rust CI/CLI logic orchestrates them via `std::process::Command`.

## Concepts

- **Store path:** A path like `/nix/store/abc123-nixos-system-25.11`
- **Closure:** A store path + all its dependencies (transitive). "Copy a closure" = copy the whole dependency tree.
- **Binary cache:** A server that stores NARs (Nix ARchive = compressed store paths). Workers fetch from cache instead of building locally.

## Cache Strategy

The cache is a shared acceleration layer, not a CI-only resource:

1. **User pushes first** — `pcr push` builds closures locally and pushes to the cache, leveraging reproducibility upfront
2. **CI pulls from cache** — when triggered by git push, CI checks the cache first. If closures are already there (from the user's push), CI skips the build
3. **CI pushes on miss** — if the cache doesn't have the closure, CI builds and pushes so workers can reuse it
4. **Workers pull** — always from cache, never build locally

This "push-first" model means CI is fast (cache hit) and workers are fast (pre-built NARs).

## Workflow Steps

### 0) Preconditions
- Flake config lives in `example/flake.nix` using the pinned `procurator` input
- CI runs on every push (triggered by repohub's `post-receive` hook)
- Workers are bare-metal NixOS hosts with `nix` and `cloud-hypervisor`

### 1) User edits and pushes
```nu
git add example/flake.nix
git commit -m "Update cluster"
pcr push   # builds closures + pushes to cache
git push   # triggers CI via post-receive hook
```

### 2) CI evaluates the blueprint
```nu
nix eval --json "example#blueprintJSON" > blueprint.json
```

### 3) CI builds closures (or pulls from cache)
```nu
for vm in control-plane-1 worker-1 worker-2 {
  let closure = (nix build --no-link --print-out-paths $"example#nixosConfigurations.($vm).config.system.build.toplevel")
  print $"($vm) => ($closure)"
}
# Nix automatically uses the binary cache — if the user already pushed, this is instant
```

### 4) CI pushes to cache (on miss)
```nu
# Only if closures weren't already cached:
nix copy --to ssh-ng://cache-host /nix/store/abc123-nixos-system-25.11
```

### 5) CI generates a deployment artifact
A JSON payload with:
- `blueprint.json` (serializable cluster topology)
- Store paths per VM (closures)
- Git commit hash

### 6) CI notifies control plane
Only if changes are detected (any VM closure path differs from previous deployment).

### 7) Control plane schedules
Compares desired closures with live systems on each worker. If store paths differ → deploy.

### 8) Workers pull and activate
```nu
nix copy --from https://cache.example.org /nix/store/abc123-nixos-system-25.11
sudo /nix/store/abc123-nixos-system-25.11/bin/switch-to-configuration test
sudo /nix/store/abc123-nixos-system-25.11/bin/switch-to-configuration boot
```

### 9) Rollback on failure
```nu
sudo nixos-rebuild switch --rollback
```

## Change Detection

- Build closures for all VMs
- Compare store paths with previous `deployment.json`
- If any path differs → changes exist → deploy
- If all match → skip

## Deployment Artifact

Stored as a CI artifact (never committed to git):
- Git commit hash
- Blueprint JSON (topology + deployment config)
- Per-VM closure store paths
- `flake.lock` pins inputs but does NOT contain outputs or store paths
