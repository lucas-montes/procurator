# SCE notes

How to use opencode?
Do I need to clone the whole repo?


# PRocurator notes
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

configs for kernel:https://github.com/cloud-hypervisor/cloud-hypervisor/issues/7058#issuecomment-2869408095

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


# GitOps-Driven Nix VM Orchestrator — End-to-End Flow

This document describes the full lifecycle of a declarative, GitOps-based VM orchestrator built on Nix and cloud-hypervisor.
There is **no imperative apply command**. Git is the only write interface to cluster state.

---

## Core Invariant

**The cluster continuously reconciles itself to a set of Nix derivations produced from a Git commit, evaluated outside the cluster, scheduled deterministically, and executed immutably.**

---

## Actors

- **User**: edits Nix configuration and pushes to Git
- **Git Server**: source of truth for desired state
- **Nix Evaluation Server / Build Farm**: evaluates and builds VM derivations
- **Master Node**: stores desired state, schedules VMs, tracks convergence
- **Worker Nodes**: reconcile assigned desired state using cloud-hypervisor
- **Binary Cache**: distributes built Nix closures

---

## High-Level Flow

Git → Nix Eval / Build → Master → Workers → cloud-hypervisor

---

## Step-by-Step Lifecycle

### 1. User declares intent (authoritative input)

**User machine**
- User edits Nix flake defining cluster VMs:
  - VM logical IDs
  - NixOS modules
  - replicas
  - labels / selectors
  - resource hints
- User commits and pushes to Git.


> Git is the only write interface to desired state.


---


### 2. Git change triggers evaluation


**Nix Evaluation Server / CI**
- Watches Git repository.
- On new commit:
  - `nix eval .#orchestrator.vms --json`
  - `nix build .#orchestrator.vmImages`
- Produces:
  - evaluated VM specs
  - derivation output store paths
  - content hashes
  - metadata (labels, resources)
- Publishes VM closures to binary cache.


---


### 3. Desired state compilation


**Nix Evaluation Server**
- Computes:
  - `intentHash = hash(commit + evaluated VM specs)`
  - monotonic `generation`
- Ensures:
  - evaluation and builds fully succeeded
  - no partial generations are published


---


### 4. Desired state publication


**Nix Evaluation Server → Master**
- Cap’n Proto RPC:
  - `PublishDesiredState`
- Payload:
  - commit hash
  - generation number
  - intent hash
  - full desired VM spec list (hash + store path)


> This replaces any `apply` command.


---


### 5. Desired state storage


**Master Node**
- Validates incoming desired state.
- Stores:
  - desired state by generation
  - commit → generation mapping
- Marks generation as “active”.


---


### 6. Deterministic scheduling


**Master Node**
- Runs pure scheduling function:

desired VMs × worker inventory → assignments

- Scheduling is:
- label-aware
- Stores per-worker desired assignments.


---


### 7. Worker reconciliation (pull-based)


**Worker Node → Master**
- Periodically (or via stream):
- `GetAssignment(workerId, lastSeenGeneration)`


**Master → Worker**
- Returns:
- current generation
- full list of desired VM specs assigned to that worker


> No push, no imperative commands.


---


### 8. Worker reconciliation loop


**Worker Node**
For each assigned VM:
- Compare:
- desired hash
- running hash
- Decide:
- noop
- stop + replace
- create


This loop runs:
- on assignment changes
- on worker restart
- periodically for drift detection


---


### 9. Nix realization on worker


**Worker Node**
- Ensure VM closure exists:
- `nix copy` from binary cache
- or local build if necessary
- Result:
- immutable VM artifact
- cloud-hypervisor config produced by derivation


Workers are fully disposable.


---


### 10. Immutable VM execution


**Worker Node**
- For any VM requiring change:
- stop old VM
- start new VM using cloud-hypervisor
- No in-place mutation.
- Replacement is hash-driven and deterministic.


---


### 11. Observed state reporting


**Worker Node → Master**
- Periodic heartbeat:
- running VM hashes
- VM status
- uptime
- resource metrics
- Includes current generation seen by worker.


---


### 12. Continuous convergence


**Master Node**
- Compares:
- desired state vs observed state
- Detects:
- drift
- lagging workers
- failed VMs
- Does not issue imperative fixes.
- Relies on worker reconciliation.


---


### 13. User observability


**User**
- Uses read-only CLI:
- `orchestrator status`
- Status reports:
- active Git commit
- generation
- convergence percentage
- per-worker and per-VM state
- drift explanations


Rollbacks are done via Git:

git revert <commit>
git push



---


## Failure Semantics


- **Worker loss**: worker restarts → pulls assignment → recreates VMs
- **Master restart**: reloads desired state → workers reconcile
- **Build failure**: generation not published → cluster unchanged
- **Partial outage**: system converges when components return


---


## Summary


- No imperative lifecycle commands
- No `apply`
- Git + Nix define truth
- Master schedules, workers reconcile
- VMs are immutable
- Convergence is continuous


This is a **GitOps-native, Nix-first VM orchestrator**.

# Procurator – Local Developer Platform Vision

This file describes the roadmap, TODOs, and UX notes for implementing Procurator with a declarative, seamless local development experience powered by Nix.

---

## Core Philosophy Notes

* Developers must never think about environments
* Local, staging, and production parity is an invariant
* The laptop is only a host that instantiates project specifications
* All configuration lives in the PROJECT repo
* CLI commands must never allow unspecified drift
* Automation should remove need for docker, kubectl, asdf, nvm, manual .env files

---

# CLI Experience Design Goals

### Golden flow

A developer workspace should feel like:

clone → open → init → up → code → test

with no other tools.

### Command expectations

| Command     | Intent                            |
| ----------- | --------------------------------- |
| init        | prepare the workspace             |
| stack up    | start the declared runtime        |
| stack down  | destroy local ephemeral resources |
| stack stop  | pause execution                   |
| stack start | resume                            |
| test        | run CI-parity tests               |
| inspect     | visualize local/remote cluster    |

---

# Implementation Roadmap

## Phase 1 – Foundations

* Clone CLI into Rust crates
* Structured logging subsystem
* CLI to agent communication
* Workspace detection logic

---

## Phase 2 – Nix Spine

* Integrate with nix flakes
* Auto-run nix develop
* Build cache design

---

## Phase 3 – Declarative Services

* Define project config format
* Repo linking

---

## Phase 4 – Local Cluster Runtime

* Embed k3s/kind equivalent
* Manifest generation
* Networking

---

## Phase 5 – VCS Integrations

* Internal git server
* Auth tokens

---

## Phase 6 – CI Parity

* Pipeline format

---

## Phase 7 – Secrets Magic

* Secure secrets store

---

# Stack Namespace Detailed TODOs

### Lifecycle

* stack up TUI supervisor
* detach mode
* graceful shutdown

---

### Logs Aggregation

* Unified logs stream similar to Procfile
* Prefix coloring
* persistent log storage

---

### Drift Monitoring

* stack check
* stack show

---

# Inspect Namespace Vision

* Implement TUI with ratatui
* Read-only dashboard

---

End of roadmap.
