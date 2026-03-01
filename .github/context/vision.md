# Project Vision

> Why procurator exists, what problem it solves, and where it's heading.

## The Problem

Modern software infrastructure requires stitching together a patchwork of loosely coupled tools: GitHub for code hosting, ArgoCD for deployment, Vault for secrets, Docker registries for images, separate CI platforms, and Kubernetes for orchestration — each with its own configuration language, failure modes, and operational surface area. Plugins and glue code proliferate. Nothing shares a common abstraction.

Nix already solves the hard part: producing bit-for-bit reproducible artifacts from declarative descriptions. NixOS extends that to entire machine configurations. What's missing is a platform that **uses Nix as the unifying substrate** for the full lifecycle — code hosting, CI, image building, caching, orchestration, and deployment — instead of bolting Nix onto tools that weren't designed for it.

## The Vision

Procurator is a **Nix-native platform** that integrates everything typically brought through external services and plugins into a single, coherent system:

- **Code hosting** (replacing GitHub) → Repohub
- **Continuous integration** (replacing GitHub Actions, Jenkins) → CI Service
- **Build caching** (replacing Cachix, S3-based caches) → Cache
- **VM orchestration** (replacing Kubernetes) → Worker + Control Plane
- **Secrets management** (replacing Vault) → future, integrated
- **Dependency & infrastructure discovery** (no mainstream equivalent) → Autonix
- **Developer workflow** (replacing docker-compose, Tilt, Skaffold) → CLI

The key difference from Kubernetes: instead of containers + YAML + a plugin ecosystem, procurator uses **Nix closures + flakes + cloud-hypervisor VMs**. A flake declares what goes into a machine, what resources it needs, what network access it gets. The platform evaluates, builds, caches, and deploys it as a VM — deterministically, reproducibly, with full-stack auditability from source to running machine.

## Components & Their Roles

### Repohub — Code Hosting + Project Management

The "GitHub" of the platform. Stores Git repositories and manages **projects** — a project is a collection of repos that form a system. Each repo has its own flake; a **master flake** links them together, declares the cluster topology, and defines service orchestration (like a Procfile).

Repohub provides a **web UI for creating and declaring projects**: adding repos, configuring the master flake, visualizing infrastructure topology and dependency graphs parsed from flakes. Autonix assists by generating per-repo flakes and discovering dependencies.

Master flake authoring has **two paths**:
- **Repohub web UI** — create a project, add repos, configure the master flake visually. Autonix fills in defaults.
- **`pcr init`** — run locally, **no repohub connection required**. Flags control scope:
  - `pcr init` in a directory of repos → generates master flake (project-level)
  - `pcr init` in a single repo → generates per-repo flake
  - Auto-detection: if the directory contains subdirectories with source code, treat as project; otherwise treat as single repo

If the user creates the project via repohub, autonix doesn't need to generate the master flake (repohub already did it). If `pcr init` is used locally, autonix handles it. The two paths are independent.

Repohub triggers CI via **post-receive hooks** after each push. Build results and test status are shown by the **CI service**, not repohub — the two can run separately.

### CI Service — Continuous Verification

Runs after every push. Its job: evaluate the flake, run `nix flake check` (or `nix build` — whichever avoids rebuilding unchanged outputs), and push results to the cache. Always a flake check — no custom pipeline YAML. CI pulls from cache when possible and pushes on miss.

The CI service **owns the build results view** — it displays per-check breakdown (which outputs passed/failed), build logs with timing, and test output. It's a library, so it can expose a web UI or an API. This is separate from repohub, so CI can run standalone (e.g., triggered by a post-receive hook on a local bare repo without repohub).

After a successful build, CI **pushes a notification to the control plane** with the new desired state. This is the reconciler's trigger — CI always knows when something changed and can compare previous vs. current state.

### Cache — Shared Build Artifacts

A Nix binary cache (nix-serve compatible) shared between users, CI, and workers. The **push-first model**: the user pushes with `pcr push`, CI pulls from cache and pushes on miss, workers pull when creating VMs. No redundant builds across the pipeline.

### Worker — VM Lifecycle Management

Manages cloud-hypervisor processes on a physical host. Receives a `VmSpec` (8 fields: kernel, initrd, disk image, cmdline, cpu, memory, allowed domains, toplevel), pulls store paths from cache, spawns the VM, and manages its lifecycle. One CH process per VM. Network isolation via host-side nftables.

The worker is **sandbox-agnostic** — it creates and destroys VMs. Whether the VM runs an LLM agent, a test suite, a web server, or a dev environment is determined by the flake, not the worker.

### Control Plane — Reconciler

Operates as a **reconciler**, not an imperative scheduler. The flake declares the desired state (which VMs, how many, where). The control plane reads it, diffs against reality, and converges — like a continuous `nixos-rebuild switch` for the cluster.

Responsibilities:
- Receive desired-state notifications from CI (the trigger for reconciliation)
- Read desired state from the evaluated cluster topology (flake)
- Diff against running VMs across workers
- Converge: create missing VMs, destroy extra ones, reschedule on failure
- Track worker capacity and health

The control plane does NOT persist state durably — desired state is always reconstructable from Git (the flake is the source of truth). This is narrower than Kubernetes: no custom resources, no admission controllers, no plugin ecosystem. Just "make reality match the flake."

### Autonix — Intelligence Layer

Parses a directory and extracts **as much information as possible** about the system. Sources include:

- Dockerfiles, docker-compose files (services, ports, dependencies)
- Package manifests (package.json, Cargo.toml, requirements.txt, go.mod)
- Lock files (package-lock.json, Cargo.lock, etc.)
- CI/CD configuration files (port mappings, service declarations)
- Standard ports for known services (PostgreSQL=5432, Redis=6379)

From this, autonix generates:

1. **Per-repo Nix flakes** — packaging each application for Nix
2. **VM images** (future) — producing bootable VM specs
3. **Cluster topologies** (future) — discovering dependencies and generating a cluster definition with all required VMs, default resources, and networking

The user can always **modify the generated values** — autonix provides a starting point, not a locked-in configuration. VM sizes, service ports, and dependency choices are all editable.

Autonix is used by both the CLI (local flake generation) and repohub (automated dependency discovery via web UI). See `autonix/MASTER_FLAKE.md` for the master flake design.

### CLI — Developer Workflow

GitOps-based developer interface. Key operations:

- **`pcr push` / `pcr pull` / `pcr clone`** — manage *projects* (a project is a collection of repositories). `pcr clone` can clone the entire project or a single repo within it.
- **`pcr run`** — **Procfile-like** experience: starts all services defined in the master flake using a **built-in Rust process manager** (process-compose-like, part of the CLI crate). Two modes:
  - **Process mode** (default): runs services as native processes — fast startup, low overhead, good for development.
  - **VM mode** (`pcr run --vm`): spins up cloud-hypervisor VMs matching production topology — full isolation, production parity.
  Full-featured process manager with:
  - **Log multiplexing** — colored per-service output (like docker-compose)
  - **TUI** — interactive dashboard showing service status, logs, health
  - **Port forwarding** — in VM mode, expose VM ports to localhost
  - **File watching** — restart services on code change (like Tilt)
  - **Dependency ordering** — start services in correct order based on declared deps
  - **Health checks** — wait for readiness before starting dependents
  - **Restart policies** — configurable per-service restart on failure
  - **Clean shutdown** — graceful stop in reverse dependency order
- **Worker testing** — `pcr-worker-test` for direct RPC calls to a worker (create-vm, delete-vm, list-vms)

The CLI never builds on the cluster — it builds locally and pushes to cache, or triggers CI to build.

### Repo Outils — Shared Utilities

Library crate with functions and structures for working with Git and Nix. Used by multiple crates (CI service, repohub, CLI) to avoid duplication.

### Commands — RPC Schemas

Cap'n Proto schemas defining the wire protocol between all services. The single source of truth for inter-service communication.

## Core Design Principles

1. **Nix is the substrate.** Every artifact — from a single package to a full VM image — is a Nix derivation. Content-addressed, reproducible, cacheable.

2. **Git is the write interface.** Desired state lives in flakes, checked into Git. No `kubectl apply`, no imperative mutations. Push a commit → the system converges.

3. **Integrated, not composed.** Repohub, CI, cache, worker — these aren't independent tools wired together with webhooks. They're crates in one workspace, sharing types and protocols, designed to work as a unit.

4. **VMs, not containers.** Cloud-hypervisor provides real hardware isolation. Each VM gets its own kernel. Network isolation is enforced at the host level. This matters for untrusted workloads (LLM agents, third-party code).

5. **The worker is dumb.** It takes a VmSpec and boots a VM. It doesn't know what runs inside. All intelligence is in the flake (what to build) and the control plane (where to run it).

6. **Library-first crates.** Each service crate (ci_service, repohub, etc.) exposes a `lib.rs` with easy-to-use interfaces. The `main.rs` is a thin binary wrapper. This enables: running everything as a **single binary** for development/testing, or deploying as separate services in production. A NixOS module will later provide `procurator.enable = true` to run all services on one machine.

7. **Push-first caching.** Users push builds to cache proactively. CI and workers pull. No build happens on the cluster if it can be avoided.

## Primary Use Cases

### 1. LLM Agent Sandboxes

A flake declares the tools, code, machine specs, and network constraints for an agent sandbox. The worker spins up a VM, the agent runs inside with everything it needs, and the VM is destroyed when done. Network-restricted by default (allowedDomains).

### 2. Deterministic Simulation Testing

Same VM lifecycle, but the workload is a test suite. The worker (or test harness) can introduce failures externally — kill processes, drop network, corrupt files — while the software inside runs as if in a regular environment. The VM is a controlled environment we own.

### 3. Self-Hosted Infrastructure

Full GitOps loop: push a flake describing your services → CI builds images → control plane schedules VMs → workers boot them. Like a self-hosted Kubernetes + ArgoCD + GitHub, but Nix-native.

## The Full Pipeline

```
Developer writes code
        │
        ▼
Autonix scans repo → generates/updates flake.nix
        │
        ▼
Developer pushes to Repohub
        │
        ▼
Post-receive hook triggers CI Service
        │
        ▼
CI evaluates flake → builds → pushes to Cache
        │
        ▼
CI pushes notification to Control Plane ("new desired state")
        │
        ▼
Control Plane schedules VMs to Workers
        │
        ▼
Workers pull from Cache → boot VMs via Cloud Hypervisor
        │
        ▼
VMs run with network isolation, secrets injection, monitoring
```

## What's Built vs. What's Planned

| Component | State | Notes |
|-----------|-------|-------|
| Worker (VM lifecycle, CH backend, mock backend) | **Working** | 22 unit tests, createVm/deleteVm/listVms RPC |
| Nix lib (profile → image → cluster → host) | **Working** | 4-layer pipeline, fast tests + integration test |
| Cap'n Proto schemas (worker + master + common) | **Working** | 8-field VmSpec, 4 worker RPCs, 5 master RPCs |
| Autonix (repo scanning, flake generation) | **Working** | Detects languages, frameworks, dependencies |
| Cache (nix-serve compatible) | **Working** | Push-first model |
| Repo Outils (git + nix utilities) | **Working** | Used by multiple crates |
| Repohub (web UI, repo management) | **Scaffolded** | Models, routes, templates exist |
| CI Service (build pipeline) | **Scaffolded** | Job queue, worker, database exist |
| Control Plane (scheduler) | **Scaffolded** | Server, node, scheduler stubs |
| CLI (project management) | **Scaffolded** | Test binaries work, interactive mode planned |
| Secrets management | **Not started** | 9p share for MVP, custom vault future |
| Cluster topology from Autonix | **Not started** | Dependency discovery → VM topology |
| Fault injection | **Not started** | External failure injection for testing |

## Project & Flake Structure

A **project** is a collection of repos that form a system:

```
my-project/
├── api/              ← repo with its own flake.nix
├── frontend/         ← repo with its own flake.nix
├── worker/           ← repo with its own flake.nix
├── infra/
│   ├── postgres.nix  ← component: PostgreSQL VM profile + specs
│   ├── redis.nix     ← component: Redis VM profile + specs
│   └── monitoring.nix
└── flake.nix         ← master flake (imports repos + components, declares cluster)
```

- Each repo has its own flake (generated by autonix or written manually)
- **Infrastructure components** (databases, caches, proxies) are defined as **separate `.nix` files** with their VM profiles, specs, and package-specific `extraConfig`, then imported by the master flake. Autonix can generate these from discovered dependencies. Example:
  ```nix
  # infra/postgres.nix
  { mkVmProfile, pkgs }: mkVmProfile {
    hostname = "postgres";
    packages = p: [ p.postgresql_16 ];
    entrypoint = "${pkgs.postgresql_16}/bin/postgres -D /var/lib/postgresql";
    cpu = 2; memoryMb = 2048;
    extraConfig = {
      databases = [ "mydb" ];
      extensions = [ "pgvector" "postgis" ];
      maxConnections = 200;
    };
  }
  ```
- The **master flake** imports all repo flakes + component files and provides:
  - Cluster topology (`evalCluster` with VMs, addresses, roles)
  - Service orchestration (Procfile-like process declarations)
  - Composed devShells
  - Unified `nix flake check` across all repos
- Projects are created/managed via **repohub's web UI** or the CLI

See `autonix/MASTER_FLAKE.md` for detailed master flake design.

## Deployment Topology

The cluster config defines **where things run** via deployment addresses:

```nix
evalCluster {
  vms = {
    api      = { vmProfile = profiles.api;    deployment.addr = "10.0.0.1"; };
    postgres = { vmProfile = profiles.pg;     deployment.addr = "10.0.0.2"; };
    redis    = { vmProfile = profiles.redis;  deployment.addr = "10.0.0.2"; };
  };
}
```

Because everything is Nix, **every service knows about the full cluster at build time**. The flake evaluation has all VM names and topology. Connection strings use **stable hostnames** (e.g., `DATABASE_URL=postgres://postgres.cluster:5432/mydb`) that are baked into the VM image by Nix.

Hostname resolution is handled by **host-side DNS** (dnsmasq on each host, already part of the networking module). The control plane updates DNS records when VMs are placed or rescheduled. This gives the best of both worlds:

- **Build-time safety** — hostnames are known at eval time, so connection strings are verified by Nix. No runtime config injection.
- **Operational flexibility** — if a VM moves to a different host/IP (failure, scaling), the control plane updates dnsmasq. No rebuild needed.
- **Security** — DNS is host-local (not exposed externally), controlled by the control plane, and only resolves cluster-internal names.

The control plane reads this topology, installs procurator services on target hosts (since they're NixOS machines managed by Nix), reconciles running VMs to match, and keeps DNS records in sync.

## Resolved Design Decisions

| Question | Decision |
|----------|----------|
| Control plane model | **Reconciler** — reads desired state from flake, diffs, converges |
| Reconciler trigger | **CI pushes** — CI notifies control plane after successful builds |
| What CI runs | **Always flake check/build** — no custom pipelines |
| CI results display | **CI service owns it** — logs, timing, per-check breakdown. Library with web UI or API. |
| Master flake authoring | **Two paths** — repohub web UI or `pcr init` locally. Autonix assists both. |
| Autonix discovery sources | **Everything parseable** — Dockerfiles, manifests, lockfiles, CI configs |
| Local dev experience | **Procfile-like** — process mode (default, fast) or VM mode (`--vm`, production parity) |
| Service discovery | **Hostname-based** — build-time hostnames baked by Nix, resolved by host-local dnsmasq managed by control plane |
| Deployment topology | **Declared in flake** — `deployment.addr` per VM, hostnames for inter-service, DNS updated by control plane |
| Process manager | **Built-in Rust** — process-compose-like, part of CLI crate. TUI, log multiplexing, port forwarding, file watching, health checks, restart policies |
| `pcr init` | **Offline-capable** — no repohub needed, flags for project vs repo, auto-detection |
| Infra components | **Separate `.nix` files** — postgres.nix, redis.nix, etc. with `extraConfig` for package-specific options, imported by master flake |
| Component library | **Deferred** — goal is auto-generation via autonix first; pre-built library maybe later |
| Single binary vs microservices | **Both** — library-first crates, single binary for dev, separate for prod |
| Secrets | **Deferred** — 9p share for MVP, full design TBD |

## Open Design Questions

- **Hostname scheme** — naming convention for cluster-internal DNS (e.g., `postgres` vs `postgres.my-project.cluster`). Decides dnsmasq zone structure.
- **Secrets management** — how secrets are provided, scoped, and delivered to VMs (brainstorm needed)
- **Multi-host bootstrapping** — how the control plane installs itself on new hosts
- **`pcr init` ↔ repohub sync** — when/how does a locally-initialized project get registered in repohub?
- **`extraConfig` schema** — how component `.nix` files declare package-specific configuration knobs

See `brainstorming.md` for the full Q&A record and detailed analysis of earlier design explorations.
