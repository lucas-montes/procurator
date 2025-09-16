look at [this microvm](https://github.com/astro/microvm.nix)
docs for [micr](https://astro.github.io/microvm.nix/cpu-emulation.html)
and also the native [way](https://discourse.nixos.org/t/is-there-a-way-to-share-a-nix-store-across-multiple-machines/18186/2)

nix ressources [here](https://ryantm.github.io/nixpkgs/builders/testers/#chap-testers)
[probably this could help to share the binary accross machines](https://nix.dev/manual/nix/2.28/command-ref/nix-copy-closure.html)

[flakes docs](https://nixos.wiki/wiki/flakes#Basic_Usage_of_Flake)
[vm options](https://mynixos.com/options/virtualisation)

check how to use nixos [vms](https://github.com/ghostty-org/ghostty/tree/main/nix)

how to use [jepsen](https://github.com/aphyr/distsys-class)
learn about [distributed](https://github.com/jepsen-io/maelstrom)

Does not evaluates the file it only serialize the derivation
```bash
nix-instantiate --eval --json cluster.nix > cluster-state.json
```

```bash
nix-instantiate --eval --json cluster.nix | jq -r . | jq .
```

To run something that is not in a flake but flakes are enabled use -f
```bash
nix build -f default.nix
```

# TODO
[] Better separate worker and control plane
[] Add monitoring to cli
[] Add a build pipeline, use fetchGithub, fetchTarball and other to get the source code. How to hide nix stuff from the user?
[] Add a testing pipeline for simple unit tests (if possible with nix)
[] Add a pre-production env where to perform DST and look for regressions (if regression observed send notifications and maybe do some automatic rollback)
[] Sync with a monitoring environement (probably look how to couple it with grafana and elk stack)
[] Use chain of command to run all the steps and coordinate workers. Steps are in order: unittest, build, pre-prod deployment, dst, generate dst report, deploy to prod or rollback
[] Find how and what are the steps that we can do with nix running commands and avoiding custom code
[] The infra may not change that often so having to evaluate all of that info might be useless find if it a problem to evaluate the nix files with the infrastructure

## Notes
Probably the infra stuff should be separated from the apps things.
