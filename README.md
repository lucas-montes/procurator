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

## Notes
You need to add yourself (or some user) to the trusted users in the nix.settings.trusted-users

# TODO

## 1. Control Plane (`procurator-control-plane`)

- [ ] Define Cap'n Proto schema for worker RPC
- [ ] Implement job queue and scheduling
- [ ] Implement worker registry and health tracking
- [ ] Implement pull-based work assignment (`GetWork()`)
- [ ] Implement status update handling (`UpdateStatus()`)
- [ ] Implement deployment state machine (rolling, blue-green, canary)
- [ ] Persist state to database (PostgreSQL)
- [ ] Implement leader election for HA
- [ ] Push notifications to workers on deployments/config changes

## 2. Worker (`procurator-worker`)

- [x] Basic builder with state machine (Initial → Built → Tested → StateSaved)
- [x] Nix flake building and testing
- [x] Runtime modules structure (application, controller, executable, oci)
- [ ] Implement Cap'n Proto RPC client
- [ ] Implement pull loop (`GetWork()` polling with backoff)
- [ ] Integrate builder with RPC to execute jobs
- [ ] Implement systemd service executor for deployments
- [ ] Implement cgroup resource limits
- [ ] Collect and push metrics to observability
- [ ] Implement health checks (HTTP/TCP/exec)
- [ ] Stream logs to observability service
- [ ] Heartbeat mechanism to signal liveness

## 3. CI/CD Service (`procurator-ci-service`)

- [x] Basic queue system (BuildQueue with SQLite)
- [x] Build status tracking (Queued, Running, Success, Failed)
- [x] Worker processing builds and storing logs
- [x] Git hook integration (post-receive hook)
- [x] Web UI (index.html with build list)
- [x] Build events API
- [ ] Parse `procurator.ci` from flakes (extend nix_parser)
- [ ] Queue jobs to control plane instead of local processing
- [ ] Implement deployment triggering on successful builds
- [ ] Implement retry logic with exponential backoff
- [ ] Report status back to Git (commit checks)
- [ ] Improve CI UI logic and error handling

## 4. Cache Service

- [ ] Implement Nix binary cache protocol (`.narinfo`, `.nar`)
- [ ] Implement upload endpoint for users/CI
- [ ] Add authentication (API keys, mTLS)
- [ ] Implement LRU eviction with size limits
- [ ] Add metrics (hit/miss ratio, storage usage)
- [ ] Support S3 backend (optional)

## 5. Secret Manager

- [ ] Implement envelope encryption (DEK + KEK)
- [ ] Create/update/delete secret endpoints
- [ ] Implement secret injection (env vars + tmpfs files)
- [ ] Implement worker authentication for secret requests (mTLS)
- [ ] Audit log secret access
- [ ] Support secret rotation

## 6. Git Service / Forge

- [x] Bare repository storage (managed via RepoManager)
- [x] Post-receive hook dispatcher embedded
- [ ] Webhook integration with CI/CD service
- [ ] Improve repository access control
- [ ] Add branch protection rules

## 7. Observability Service

- [ ] Receive metrics from workers
- [ ] Store time-series data (WAL-based or InfluxDB)
- [ ] Query API for metrics
- [ ] Aggregate logs from workers
- [ ] Track service uptime
- [ ] (Phase 2) Alerting system

## 8. Web UI

- [ ] Service status dashboard
- [ ] Deployment management UI
- [x] CI pipeline visualization (basic)
- [ ] Improve code diff visualization
- [ ] Logs viewer with filtering
- [ ] Metrics/uptime graphs

## 9. Flake Schema & CLI

- [ ] Define `procurator.services.*` schema (extend infrastructure.nix model)
- [ ] Define `procurator.ci.*` schema
- [ ] Create `procurator` CLI tool
  - [x] Basic structure with commands
  - [x] Apply command scaffolding
  - [ ] `procurator deploy`
  - [ ] `procurator status`
  - [ ] `procurator logs`
  - [ ] `procurator secrets`
  - [x] Monitor command with TUI (interactive app)

## 10. NixOS Integration

- [ ] NixOS module for control plane
- [ ] NixOS module for worker
- [ ] NixOS module for cache service
- [ ] Deployment documentation

## 11. Security & Hardening

- [ ] mTLS setup (CA, cert rotation)
- [ ] RBAC (admin, developer, viewer roles)
- [ ] API authentication (tokens)
- [ ] Audit logging


## Notes
Probably the infra stuff should be separated from the apps things.

┌─────────────────────────────────────────────────────────────────┐
│                         User's Machine                          │
│  ┌──────────────┐        ┌─────────────────┐                    │V
│  │ Nix Build    │───────▶│  Cache Client   │                    │
│  │ (local dev)  │        │  (sync daemon)  │                    │
│  └──────────────┘        └────────┬────────┘                    │
└───────────────────────────────────┼─────────────────────────────┘
                                    │ push + cache upload
                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Procurator Platform                        │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  1. Git Service (forge)                                  │   │
│  │     - Git hosting + webhooks                             │   │
│  │     - Trigger CI on push                                 │   │
│  └──────────────┬───────────────────────────────────────────┘   │
│                 │                                               │
│  ┌──────────────▼───────────────────────────────────────────┐   │
│  │  2. CI/CD Service                                        │   │
│  │     - Parse procurator.* outputs                         │   │
│  │     - Queue jobs (flake check, build)                    │   │
│  │     - Deployment decisions                               │   │
│  └──────────────┬───────────────────────────────────────────┘   │
│                 │                                               │
│  ┌──────────────▼───────────────────────────────────────────┐   │
│  │  3. Cache Service (nix binary cache)                     │   │
│  │     - Store/serve .narinfo + .nar files                  │   │
│  │     - Content-addressed storage                          │   │
│  │     - Serve to: CI, workers, users                       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  4. Control Plane                                        │   │
│  │     - Scheduling (job → worker)                          │   │
│  │     - Resource management                                │   │
│  │     - Health tracking                                    │   │
│  └──────────────┬───────────────────────────────────────────┘   │
│                 │                                               │
│  ┌──────────────▼───────────────────────────────────────────┐   │
│  │  5. Workers (N nodes)                                    │   │
│  │     - Execute builds (with cache access)                 │   │
│  │     - Run services (systemd + cgroups)                   │   │
│  │     - Collect metrics → Observability                    │   │
│  └──────────────┬───────────────────────────────────────────┘   │
│                 │                                               │
│  ┌──────────────▼───────────────────────────────────────────┐   │
│  │  6. Observability Service                                │   │
│  │     - Time-series metrics storage (custom)               │   │
│  │     - Logs aggregation                                   │   │
│  │     - Uptime monitoring                                  │   │
│  │     - Dashboard UI                                       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  7. Secret Manager                                       │   │
│  │     - Encrypted key-value store                          │   │
│  │     - Injection to workers at runtime                    │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
