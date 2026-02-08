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
