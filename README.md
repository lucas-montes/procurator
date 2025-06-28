look at [this microvm](https://github.com/astro/microvm.nix)

Evaluate some file
```bash
nix-instantiate --eval --json cluster.nix > cluster-state.json
```

To run something that is not in a flake but flakes are enabled
```bash
nix build -f default.nix
```

# TODO
[] Better separate worker and control plane
[] Add monitoring to cli
[] Add a build pipeline
[] Add a testing pipeline for simple unit tests
[] Add a pre-production env where to perform DST and look for regressions (if regression observed send notifications and maybe do some automatic rollback)
[] Sync with a monitoring environement (probably look how to couple it with grafana and elk stack)
