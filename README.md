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


[this vmm seems to have a nicer api than cloud-hypervisor](https://github.com/kata-containers/kata-containers/blob/main/src/dragonball/src/api/v1/vmm_action.rs)

more things to check:

https://github.com/DarthSim/overmind
https://github.com/ddollar/foreman
https://github.com/juspay/superposition
https://github.com/juspay/services-flake
https://github.com/juspay/omnix
https://github.com/hercules-ci/flake-parts
https://github.com/Platonic-Systems/process-compose-flake/blob/main/example/flake.nix

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

## Notes
You need to add yourself (or some user) to the trusted users in the nix.settings.trusted-users

We don't need the cli to have an apply command, let's use gitops practices to have the repohub, or some other service to run the nix commands to make derivations and all the shenanigans so we can send paths and hashes to the control-plane


To manage the state of the cluster, should we go pull or push based?
A pull based makes it "easier", as we just send a new derivation or nar to the build cache and nodes pull the configs, no need to have a master that looks over them.
However with a push basaed approach we have something more deterministic, we can have rollbacks if some node fail because we'll have a confirmation of the switch.

## Components
*Project*: A set of services or repos. For a SOA (service oriented architecture) we would probably have multiple services separated in different repos
*Repo*: A regular repo, with one or more services in it. If can be a monorepo with multiple services or a single service that can run independently.

The idea is to be able to map all the services in a large project together. Get as much information automatically

## TODO

### Documentation
An agent or some AI bullshit that reads everything and writes and keeps documentation up to date.

### CLI
A cli to manage all this crap
Probably a nice tui to see things and play with them
- [ ] Set up the config from autonix
- [ ] Spin up agents
- [ ] Have a Procfile like service to run everything needed and control it easily.

### Autonix
Detec config, services and everything needed to run repos and projects.

### Control plane
for :
- Agents
- Actual servers running code
Maybe those could be the exact same thing?
yeah, it would be similar to the testing thing. A master control plane that controls machines, then another control plane that controls logic to retry and things like that

### Workers
The actual machines where the code or agents are running.
They could be a microVm or systemd
MicroVm is probably the best idea for starters so we can use it with agents.
The workers would be created from a flake
They should publish metrics

### CI/CD Service
Some pipeline to run tests in the code itself, linting validations and things like so.
It will reuse the build from the build service/registry.
If requested it should deploy things to staging

### Build/cache registry
It listents to build sent by the users local build cache

- [ ] Implement Nix binary cache protocol (`.narinfo`, `.nar`)
- [ ] Implement upload endpoint for users/CI
- [ ] Add authentication (API keys, mTLS)
- [ ] Implement LRU eviction with size limits
- [ ] Add metrics (hit/miss ratio, storage usage)
- [ ] Support S3 backend (optional)

### Secret Manager

- [ ] Implement envelope encryption (DEK + KEK)
- [ ] Create/update/delete secret endpoints
- [ ] Implement secret injection (env vars + tmpfs files)
- [ ] Implement worker authentication for secret requests (mTLS)
- [ ] Audit log secret access
- [ ] Support secret rotation

### Git Service / Forge

- [x] Bare repository storage (managed via RepoManager)
- [x] Post-receive hook dispatcher embedded
- [ ] Webhook integration with CI/CD service
- [ ] Improve repository access control
- [ ] Add branch protection rules



## Diagram
┌─────────────────────────────────────────────────┐
│                    USER                         │
│                                                 │
│    CLI  /  TUI  /  Web Interface                │
└──────────────────┬──────────────────────────────┘
                   │
                   │ RPC (Cap'n Proto)
                   │
┌──────────────────▼───────────────────────────────┐
│              MASTER NODE                         │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │         Control Plane (Master)             │  │
│  │                                            │  │
│  │  - API Server (user requests)              │  │
│  │  - Scheduler (decide which worker)         │  │
│  │  - Global state (all VMs, workers)         │  │
│  │  - Worker registry                         │  │
│  └────────────────────────────────────────────┘  │
└──────────────────┬───────────────────────────────┘
                   │
                   │ RPC commands
          ┌────────┴─────────┐
          │                  │
    ┌─────▼──────┐     ┌─────▼──────┐
    │  WORKER 1  │     │  WORKER 2  │
    │            │     │            │
    │  ┌──────┐  │     │  ┌──────┐  │
    │  │ CP   │  │     │  │ CP   │  │  Control Plane (Worker Agent)
    │  │Agent │  │     │  │Agent │  │  - Receive commands from master
    │  └───┬──┘  │     │  └───┬──┘  │  - Manage local cloud-hypervisor
    │      │     │     │      │     │  - Report status/metrics
    │  ┌───▼──┐  │     │  ┌───▼──┐  │  - Local state
    │  │cloud-│  │     │  │cloud-│  │
    │  │hyper-│  │     │  │hyper-│  │
    │  │visor │  │     │  │visor │  │
    │  └───┬──┘  │     │  └───┬──┘  │
    │      │     │     │      │     │
    │   ┌──▼──┐  │     │   ┌──▼──┐  │
    │   │ VM  │  │     │   │ VM  │  │
    │   │ VM  │  │     │   │ VM  │  │
    │   │ VM  │  │     │   │ VM  │  │
    │   └─────┘  │     │   └─────┘  │
    └────────────┘     └────────────┘
