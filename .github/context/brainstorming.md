# Brainstorming: LLM Agent Sandboxes & General VM Orchestration

> Living document — captures the evolving architecture vision, Q&A, open questions, and design constraints.

## Vision

Procurator is a Nix-native VM orchestrator whose **primary use case** is creating sandboxed environments for LLM agents. A flake declares the tools, code, machine specs, and network constraints for a sandbox. The worker spins up a Cloud Hypervisor VM, the agent runs inside it with everything it needs, and the VM is destroyed when done.

**Secondary use case:** Deterministic simulation testing and fault injection. The same worker spins up VMs for software testing declared via flakes. Because we own the VM, we can introduce failures ourselves (kill processes, drop network, corrupt files) while the code inside runs as if in a regular environment. The lifecycle is identical — create VM from flake, run workload, destroy VM. The orchestrator doesn't care what runs inside; it just provides sandboxed, reproducible, Nix-declared VMs.

**Key insight:** Both use cases share the same primitives:
- Flake → VmSpec (kernel, disk, initrd, cmdline, network constraints)
- Worker creates/manages/destroys VMs
- VM is ephemeral, reproducible, isolated
- The orchestrator manages the sandbox lifecycle, not what happens inside

## Q&A Record

### 1. Workflow — Who triggers sandbox creation?

**Answer:** Triggered from GitHub PRs, but also manually or programmatically by scripts. There is an **agent setup declaration** that specifies:
- VM specs (machine resources)
- Which LLM agent to run in the sandbox
- Network limitations
- The commit, branch, or repo to work on

**Implication:** Need an API/trigger layer that accepts a session request (agent setup declaration) and turns it into a VM lifecycle. The trigger source is pluggable — GitHub webhook, CLI, HTTP API, etc.

### 2. Can the same logic handle deterministic simulation testing?

**Answer:** Yes. The worker doesn't care what runs inside the VM. A testing flake declares the test environment just like an agent flake declares the agent environment. Same create/run/destroy lifecycle.

**Implication:** The worker's API is generic — it takes a VmSpec and manages the VM. The "meaning" of the VM (agent sandbox vs test runner) is determined by the flake contents, not by the worker.

### 3. Task duration?

**Answer:** Long-running tasks. An agent session may run for minutes to hours (exploring code, running tests, writing PRs).

**Implication:** Need monitoring, log streaming, and possibly heartbeat/timeout mechanisms. Can't just fire-and-forget.

### 4. VM lifecycle — create and destroy only?

**Answer:** Yes, for now. Just create and destroy. No pause/resume, no snapshots, no migration.

**Implication:** Simplifies the worker API. The VmSpec + create + destroy is the MVP. Pause/resume/snapshot can come later via the existing `Vmm` trait methods.

### 5. Multiple agents in one sandbox?

**Answer:** Possible but not our concern. Our job is the sandbox (the VM). What runs inside is the user's business.

**Implication:** The worker manages VMs, not agents. A VM might have one agent or five — the worker doesn't know or care.

### 6. Agent interaction — how does the user talk to the agent?

**Answer:** The agent is inside the VM and has everything it needs. But the user may want to:
- Talk to the agent (send input, approve actions)
- Stream logs to the outside world

SSH is one option. Looking for the easiest solution.

**Implication:** This maps to the diagram's "Session API and Realtime Stream" layer. Options:
- **SSH:** Already supported by `nix/flake-vmm/vm-module.nix`. Worker knows the VM's IP. Expose SSH port or proxy it.
- **vsock:** Virtio socket — no network required, direct host-guest channel. CH supports it. More secure, doesn't depend on guest networking. Requires a guest-side daemon.
- **Serial console:** Already connected (console=ttyS0). Good for logs, awkward for interactive use.

**Recommendation (to explore):** vsock for the control channel (input/approval/status), serial for raw log streaming, SSH for interactive debugging. But SSH is the simplest starting point since it's already in the VM images.

### 7. Network access?

**Answer:** VMs may need network access, but it should be **restricted**.

**Implication:** Already solved by `host-module.nix` — nftables domain allowlisting. The flake's `allowedDomains` field maps directly to per-VM nftables rules. An agent sandbox that needs `api.openai.com` and `github.com` but nothing else gets exactly that.

### 8. Secrets?

**Answer:** Yes, VMs may need to fetch secrets (API keys, tokens, etc.).

**Implication:** Need a secrets delivery mechanism. Options:
- **Inject at build time via `files`:** The `mkVmImage { files = { "/run/secrets/openai" = "sk-..."; }; }` pattern. Problem: secrets end up in the Nix store (content-addressed, world-readable).
- **Inject at boot via metadata/cloud-init:** Worker writes secrets to a virtio-blk device or 9p share that the VM mounts. Secrets stay out of the Nix store.
- **Fetch from a secrets store at runtime:** VM has a scoped token, calls out to Vault/AWS SSM/a custom secrets API. Requires network access to the secrets store.
- **vsock-based secrets API:** Guest daemon requests secrets via vsock → host-side worker serves them from a local secrets store. No network dependency.

**Recommendation (to explore):** vsock-based or 9p/virtio-blk injection. Avoid Nix store for secrets.

### 9. Flake layering?

**Answer:** Multiple layers:
1. A **project flake** packages the application/code
2. An **agent flake** declares infrastructure + LLM agent config + uses the project flake as a dependency

The agent flake imports the project flake as a package, giving the agent sandbox access to the built project.

**Implication:** This is standard Nix flake composition. The agent flake's `inputs` references the project flake. `mkVmFromDrv` already supports this — the `drv` argument can be any derivation, including one from an upstream flake. Example:

```nix
# agent-sandbox.flake.nix
{
  inputs = {
    ch-vmm.url = "path:../../nix/flake-vmm";
    my-project.url = "github:org/repo/feature-branch";
  };
  outputs = { ch-vmm, my-project, ... }: {
    packages.x86_64-linux.sandbox = ch-vmm.lib.x86_64-linux.mkVmImage {
      extraPackages = p: [
        my-project.packages.x86_64-linux.default  # the project
        p.python3                                   # LLM agent runtime
        p.git p.gh                                  # tools the agent needs
        # ... LSPs, build tools, etc.
      ];
      entrypoint = "/run/agent/start.sh";
      allowedDomains = [ "api.openai.com" "github.com" ];
      sshAuthorizedKeys = [ "ssh-ed25519 AAAA..." ];
      files = {
        "/run/agent/start.sh" = ''
          #!/bin/sh
          # Start the LLM agent daemon
          exec /path/to/agent --project /nix/store/.../my-project --repo /workspace
        '';
      };
    };
  };
}
```

### 10. API or lower-level integration?

**Answer:** Lower-level (Temporal is the orchestration example from the diagram). Open to suggestions for simplification.

### 11. Concurrent sandbox count?

**Answer:** "Does it matter? What would it change?"

**Response:** It matters for:
- **Resource management:** Each VM consumes CPU, RAM, disk. The worker needs resource accounting to avoid overcommit.
- **Scheduling:** If demand exceeds one host, the control plane schedules across workers. But for now (single host), it's just resource limits.
- **Practical answer:** Start without limits. CH processes are lightweight (~20MB RSS idle). A 64GB host can run 20-30 VMs with 2GB RAM each. Add resource tracking later.

### 12. VM placement?

**Answer:** Sandboxes (VMs) run on the same host as the worker.

**Implication:** No network-based VM placement needed (yet). The worker is the hypervisor host. Simplifies everything — no remote storage, no image transfer latency, Nix store is local.

---

## Follow-Up Q&A (Round 2)

### F1: Testing use case — fault injection model

**Question:** Do you need the worker to report back the VM's exit code or extract files after shutdown?

**Answer:** The testing model is: spin up a VM with some specs, and since we own the VM we can introduce failures ourselves. The code inside runs as if it was a regular environment. The worker should collect results and push them to the master (or somewhere) periodically.

**Implications:**
- The worker needs a **result collection** mechanism — not just fire-and-forget. Periodically collect status/results from running VMs and report them upstream.
- Fault injection is external to the VM (the worker or test harness introduces failures via CH API, network manipulation, etc.) while the software under test runs unaware inside.
- This means the worker has a dual role for test VMs: lifecycle manager AND fault injector / observer.
- Need a reporting channel: worker → master (or storage). Periodic push of VM status, test results, metrics.

### F2: Secrets — simplest approach first

**Question:** Where do secrets live today?

**Answer:** Greenfield. Long-term goal is a custom vault-like solution. For now, go with the simplest approach.

**Decision:** For MVP, use **environment files injected via 9p share**. Worker writes secrets to a host directory, mounts it into the VM as a 9p virtio filesystem. The VM reads secrets from a known mount point (e.g., `/run/secrets/`). No guest-side daemon needed — CH supports 9p natively.

Later: build a custom secrets service (vsock-based or HTTP-based) that the VM can query at runtime.

### F3: Who builds the VM image?

**Question:** CI pre-builds, worker builds on-demand, or separate build service?

**Answer:** Separate build service. Could be the user, `ci_service`, or a build farm. The worker should just pull pre-built images.

**Decision:** The worker receives a VmSpec with Nix store paths that are already built. It pulls them via `nix copy --from <cache>`. The worker never runs `nix build`.

**Push vs Pull:** The worker **pulls** from the binary cache. This is more natural:
- Worker decides when to fetch (on create-vm)
- No coupling between builder and worker
- `nix copy --from <cache> <store-path>` is idempotent and handles deduplication
- Multiple workers can pull the same image independently

### F4: Log streaming

**Question:** How to get logs out of the VM?

**Answer:** Tools like OpenCode are accessible through API endpoints and SSH. We just need something open in the VM so we can retrieve logs inside.

**Decision:** The agent/tool inside the VM exposes its own access mechanism (API endpoint, SSH, etc.). The worker's job is:
1. Ensure the VM has network connectivity to the host (TAP + bridge — already exists)
2. Know the VM's IP address
3. Optionally proxy or expose the VM's ports

The worker does NOT need to implement log streaming itself. The agent software handles it. The worker just needs to report the VM's IP/port so clients can connect directly.

For serial console: still useful as a fallback/debug channel. CH exposes it via PTY.

### F5: Control plane — not needed for MVP

**Question:** Do you actually need the control plane for MVP?

**Answer:** No. Focus on the worker only. The CLI is for testing the worker. At the end, using a CLI to make requests or a separate service is the same — the worker's RPC API is the interface.

**Decision:** MVP architecture is:
```
CLI (pcr-worker-test) ──capnp──▶ Worker ──CH REST──▶ cloud-hypervisor VMs
```
No control plane, no API gateway, no session orchestration. Just:
1. CLI sends createVm/deleteVm/listVms/read to the worker
2. Worker manages VM lifecycle via CH
3. Worker pulls Nix closures from cache when creating VMs

The control plane, webhook triggers, and session orchestration are future layers that sit in front of the same worker RPC API.

---

## Follow-Up Q&A (Round 3)

### F6: Result collection — what's easily available?

**Question:** What information do we have easily available for result reporting?

**Answer:** Keep it to the simplest possible for now.

**What CH gives us for free (via `GET /api/v1/vm.info`):**
- `state`: "Created", "Running", "Shutdown", "Paused" — the CH process state
- `config`: the full `ChVmConfig` (CPUs, memory, disk, kernel, etc.)
- `memory_actual_size`: actual memory in bytes (optional)

**What CH gives us via `GET /api/v1/vm.counters`:**
- `block`: read_bytes, write_bytes
- `net`: rx_bytes, tx_bytes

**What the worker already tracks internally:**
- `VmStatus` enum: `Creating`, `Running`, `Paused`, `Stopping`, `Stopped`, `Failed(String)`
- `VmMetrics`: cpu_usage, memory_usage, network_rx/tx_bytes (struct exists but not populated yet)
- VM ID, content hash, uptime (via `RunningVm` in capnp schema)

**Decision for MVP:** The `listVms` RPC already returns `VmStatus` per VM. For now:
1. Worker polls CH `vm.info` to get the `state` — maps to our `VmStatus` enum
2. Worker polls CH `vm.counters` for basic I/O metrics
3. `listVms` returns this data to the CLI
4. No periodic push anywhere — the CLI (or future control plane) polls the worker

This is enough for testing. Richer reporting (exit codes, artifact extraction, periodic push) comes later.

### F7: How can the CLI give the worker everything it needs without building locally?

**Question:** How can the worker get everything it needs to build the VM without having to build the flake locally?

**This is the key architecture question.** Here's how the pipeline works:

#### The Problem

The `vmSpec` in the flake (lines 270-279 of `flake-vmm/flake.nix`) contains store paths:
```nix
vmSpec = {
  hostname = "...";
  kernel = "${nixos.config.system.build.kernel}/.../bzImage";  # /nix/store/...-linux-6.6/bzImage
  initrd = "${nixos.config.system.build.initialRamdisk}/.../initrd";  # /nix/store/...-initrd-linux-6.6/initrd
  image = <the ext4 disk image derivation>;  # /nix/store/...-nixos-ext4.img
  allowedDomains = [ ... ];
  cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
};
```

These are **Nix store paths** — they only exist after `nix build` runs. `nix eval` **cannot** resolve them to concrete `/nix/store/...` paths because they involve string interpolation of derivation outputs (IFD — Import From Derivation).

#### The Solution: Build Elsewhere, Send Store Paths to Worker

The flow is:

```
┌─────────────────┐    ┌───────────────────┐    ┌──────────────┐
│ Build Service   │    │ CLI / Trigger     │    │ Worker       │
│ (user, CI, farm)│    │                   │    │              │
│                 │    │                   │    │              │
│ nix build       │    │                   │    │              │
│ .#vmSpec        │───▶│ receives store    │───▶│ nix copy     │
│                 │    │ paths (JSON)      │    │ from cache   │
│ pushes to       │    │                   │    │              │
│ binary cache    │    │ sends VmSpec      │    │ spawns CH    │
│                 │    │ via capnp RPC     │    │ with paths   │
└─────────────────┘    └───────────────────┘    └──────────────┘
```

**Step 1 — Build (done externally):**
```bash
# On the build machine (user laptop, CI, build farm):
nix build .#my-sandbox.vmSpec --json
# This builds everything and outputs the vmSpec attrset with resolved store paths.
# Then push to a binary cache:
nix copy --to s3://my-cache .#my-sandbox.image
nix copy --to s3://my-cache .#my-sandbox.nixos.config.system.build.kernel
nix copy --to s3://my-cache .#my-sandbox.nixos.config.system.build.initialRamdisk
```

**But wait** — `vmSpec` is an attrset, not a derivation. You can't `nix build` an attrset. You need to either:

**(a) Use `nix eval` after building the dependencies:**
```bash
# Build the actual derivations first
nix build .#my-sandbox.image .#my-sandbox.nixos.config.system.build.kernel .#my-sandbox.nixos.config.system.build.initialRamdisk
# Then eval the vmSpec (now IFD is satisfied, store paths resolve)
nix eval --json .#my-sandbox.vmSpec
# → {"hostname":"my-sandbox","kernel":"/nix/store/...-linux-6.6/bzImage","initrd":"/nix/store/...-initrd/initrd","image":"/nix/store/...-ext4.img","allowedDomains":["api.openai.com"],"cmdline":"console=ttyS0 root=/dev/vda rw init=/sbin/init"}
```

**(b) Create a `vmSpecJson` derivation in the flake** that writes the vmSpec to a JSON file:
```nix
vmSpecJson = pkgs.writeText "vm-spec.json" (builtins.toJSON vmSpec);
```
Then `nix build .#vmSpecJson` produces a `/nix/store/...-vm-spec.json` that contains all the resolved paths. This is buildable, cacheable, and pushable.

**(c) Simplest: the CLI just knows the paths.** For MVP testing, the user builds locally and provides paths:
```bash
# User builds the image locally:
nix build ./my-sandbox#image --print-out-paths
# → /nix/store/abc...-nixos-ext4.img
nix eval --json ./my-sandbox#vmSpec
# → {"kernel":"/nix/store/...","initrd":"/nix/store/...","image":"/nix/store/...","cmdline":"..."}

# User tells the CLI:
pcr-worker-test create-vm --spec-json '{"kernel":"/nix/store/...","initrd":"/nix/store/...","image":"/nix/store/...","cmdline":"...","cpu":1,"memoryBytes":536870912}'
```

#### Decision for MVP

**Option (c) — the CLI takes a JSON blob or individual args.** The user (or a script) builds elsewhere and provides the resolved store paths.

Two CLI modes:
1. **`--spec-json <json>`** — pass a complete VmSpec as JSON (matches nix eval output)
2. **`--kernel <path> --disk <path> [--initrd <path>] [--cmdline <str>] [--cpu <n>] [--memory <bytes>]`** — individual args for quick testing

The CLI constructs a capnp `VmSpec`, sends `createVm` RPC to the worker. The worker assumes the store paths are already available locally (same host) or runs `nix copy` to pull them.

**For production (future):** Option (b) — add `vmSpecJson` to the flake. The build service builds it, pushes to cache, and the trigger layer reads the JSON to construct the RPC call. The worker pulls the actual artifacts via `nix copy --from <cache>`.

---

## Architecture Analysis from Diagram

The user's diagram shows a mature agent platform architecture. Here's how it maps to Procurator:

### Layers Identified in Diagram

#### 1. Clients (Bottom)
- **Slack Bot**, **Web App**, **CLI / API Client**
- These trigger sessions and receive progress events
- **Procurator mapping:** The trigger layer. GitHub PR webhook = one client. CLI = another. Web dashboard = future.

#### 2. API Gateway (Middle)
- Receives: Start Workflow, Signal (input/stop/approve), Query Status
- Connects to: Auth and Policy
- **Procurator mapping:** Could be a thin HTTP/REST service in front of the control plane. Or the control plane itself exposes an HTTP API alongside capnp RPC.

#### 3. Temporal Control Plane (Left)
- **Task Queues** + **Workflow: InspectSessionWorkflow** + **Temporal Worker**
- Orchestrates the session lifecycle as a durable workflow
- Sends "Provision or Lease VM" and "Cleanup VM" to the VM Sandbox
- **Procurator mapping:** This is the **control plane** crate's future role. Today it's a stub scheduler. The diagram suggests using Temporal (or a Temporal-like durable workflow engine). Questions:
  - Do we embed workflow logic in Rust (state machine in the control plane)?
  - Or use an external Temporal server and write Temporal workers in Rust?
  - Or keep it simple — a request queue in the control plane that drives create/destroy?

#### 4. Execution Plane (Top — NixOS VMs)
- **VM Sandbox (NixOS)** — the VM itself
- **Agent Runner / VM Daemon** — process inside the VM that runs the agent
- Connects to:
  - **Tooling** (gcc, git, bash, cargo, go, node)
  - **LSPs** (rust-analyzer, gopls, tsserver)
  - **Ephemeral DB** (per-session SQLite or similar)
  - **AgentFS** (read/write filesystem for the agent)
- Agent actions:
  - Clone and Push (git operations)
  - Store Large Artifacts → Object Storage (R2/S3)
  - Fetch Scoped Tokens → Secrets Store
  - Open PR → GitHub
  - Stream Logs and Status → back to the control plane
- **Procurator mapping:** This is entirely inside the VM. The flake declares all of this:
  - `extraPackages` = tooling + LSPs
  - `entrypoint` = the agent runner daemon
  - `files` = agent config, workspace setup scripts
  - `allowedDomains` = network restrictions
  - The agent runner is the user's software, not ours

#### 5. State Plane (Right)
- **Turso (AgentFS DB per Session)** — per-session database
- **Object Storage (R2/S3)** — large artifact storage
- **Secrets Store** — scoped credentials
- **Procurator mapping:** External services the VM accesses via network (through allowed domains). Not part of Procurator itself, but Procurator enables access via network policy.

#### 6. Code Hosting (Right)
- **Pull Requests → GitHub**
- The agent's output: code changes pushed as PRs
- **Procurator mapping:** The VM has git + gh CLI. The agent clones, modifies, pushes. Procurator just provides the sandbox.

#### 7. Realtime Stream (Right)
- **WebSocket / SSE** — progress events from agent to clients
- **Procurator mapping:** Log streaming from VM to outside world. Options: vsock → worker → WebSocket, or SSH-based log tailing, or serial console forwarding.

### What Procurator Owns vs. Doesn't Own

| Concern | Owner | Notes |
|---------|-------|-------|
| VM lifecycle (create/destroy) | **Procurator (worker)** | Core responsibility — MVP focus |
| VM image building | **Nix (flake-vmm) / Build service** | Worker pulls pre-built images, never builds |
| Network isolation | **Procurator (host-module.nix)** | Already exists, needs wiring to worker |
| Agent software | **User** | Declared in the flake, runs inside VM |
| Log/agent access | **Agent software (inside VM)** | Exposes its own API/SSH; worker just reports VM IP |
| Secrets delivery | **Worker (9p share)** | MVP: host dir mounted as virtio-9p. Future: custom vault |
| Result collection | **Worker (future)** | Periodic status push to upstream. MVP: CLI queries listVms |
| Fault injection | **Worker or test harness (future)** | For simulation testing. Not MVP |
| Session orchestration | **Control plane (future)** | Deferred — not needed for MVP |
| Trigger/webhook handling | **API layer (future)** | Deferred — CLI is the trigger for MVP |
| Object storage | **External** | R2/S3, accessed by VM via network |
| Code hosting | **External** | GitHub, accessed by VM via network |
| Auth/policy | **TBD (future)** | Who can create sandboxes? Token scoping? |

---

## Design Principles (Emerging)

1. **The worker is sandbox-agnostic.** It creates/destroys VMs from VmSpecs. It doesn't know if it's an LLM agent, a test runner, or a dev environment.

2. **The flake is the contract.** Everything about a sandbox — tools, code, agent, network, machine specs — is declared in a Nix flake. The orchestrator's job is to evaluate, build, and instantiate that flake as a VM.

3. **Layered flakes for composition.** Project flake (code) → Agent flake (infra + agent + project-as-dep). This is standard Nix flake composition.

4. **The orchestrator manages lifecycle, not behavior.** Create, monitor (health/logs), destroy. What happens inside the VM is the agent's business.

5. **Network-restricted by default.** VMs get only the domains they declare. Defense in depth for LLM agents that might try to exfiltrate data.

6. **Secrets never in the Nix store.** Injected at runtime via 9p share (MVP) or custom vault (future).

7. **Worker never builds.** It pulls pre-built closures from a binary cache. Building is someone else's job (user, CI, build farm).

8. **The VM exposes its own access.** The worker provides network + IP reporting. The agent/tool inside the VM exposes its own API endpoint, SSH, or other interface. The worker doesn't proxy application-level traffic.

9. **Fault injection is external.** For simulation testing, the worker (or test harness) introduces failures from outside the VM (nftables, CH API, SSH) while the software inside runs unaware. The VM is a controlled environment we own.

---

## Procurator's Scope (Refined)

Given the diagram and answers, Procurator's scope is:

### MVP Scope (Worker-Only)

```
┌───────────────────────────────────────────────────────────────┐
│                    Procurator MVP                              │
│                                                               │
│  ┌────────────────┐        ┌─────────────────────────────┐   │
│  │ CLI             │ capnp  │ Worker                      │   │
│  │ (pcr-worker-test│───────▶│                             │   │
│  │                 │        │ createVm / deleteVm / list  │   │
│  │ create-vm       │        │ nix copy (pull closures)    │   │
│  │ delete-vm       │        │ CH process management       │   │
│  │ list-vms        │        │ 9p secrets mount            │   │
│  │ read             │        │ TAP + nftables networking   │   │
│  └────────────────┘        └──────────┬──────────────────┘   │
│                                        │ spawn + REST          │
│                              ┌─────────▼───────────┐          │
│                              │ cloud-hypervisor ×N  │          │
│                              │ (one process per VM) │          │
│                              └─────────────────────┘          │
│                                                               │
│  ┌───────────────────────────────────────────────────────┐   │
│  │ Nix (flake-vmm)                                       │   │
│  │ mkVmImage / mkVmFromDrv → kernel + disk + initrd     │   │
│  │ host-module.nix → bridge, TAP, NAT, domain allowlist  │   │
│  │ vm-module.nix → systemd, SSH, serial, workload entry  │   │
│  └───────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────┘
```

### Full Vision (Future)

```
┌─────────────────────────────────────────────────────────────────┐
│                        Procurator (Full)                        │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐    │
│  │ API / Trigger │  │ Control Plane│  │ Worker             │    │
│  │ (HTTP/webhook)│→│ (session     │→│ (VM lifecycle on   │    │
│  │               │  │  queue +     │  │  a single host)    │    │
│  │ GitHub PR     │  │  scheduling) │  │                    │    │
│  │ CLI           │  │  result      │  │ create/destroy VM  │    │
│  │ API client    │  │  collection) │  │ network isolation  │    │
│  └──────────────┘  └──────────────┘  │ secrets injection  │    │
│                                       │ fault injection    │    │
│                                       │ result reporting   │    │
│                                       └────────────────────┘    │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Nix (flake-vmm) + Build Service / CI                     │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                     NOT Procurator (External)                   │
│                                                                 │
│  Agent software (runs inside VM, user-provided)                 │
│  Object storage (R2/S3)                                         │
│  Secrets store (custom vault, future)                            │
│  Code hosting (GitHub)                                          │
│  LLM APIs (OpenAI, Anthropic, etc.)                             │
│  Client apps (Slack bot, web dashboard)                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## Open Questions (Remaining)

### Q1: Session Orchestration — Deferred

~~State Machine vs. External Workflow Engine?~~ **Deferred.** No control plane for MVP. The worker is the only component. Session orchestration (Temporal, embedded state machine, job queue) is a future concern for when we add the control plane layer.

### Q2: Agent Setup Declaration Format — Answered

**Decision:** For MVP, the CLI user provides store paths directly (JSON blob or individual args). The user builds elsewhere (`nix build` + `nix eval --json .#vmSpec`) and gives the CLI the resolved paths.

For production: add a `vmSpecJson` derivation to the flake that the build service builds and pushes to cache. The trigger layer reads the JSON and sends the RPC.

### Q3: Log Streaming — Answered

**Decision:** The agent software inside the VM exposes its own access mechanism (API endpoint, SSH). The worker just provides network connectivity and reports the VM's IP. Serial console is a debug fallback. No worker-side log proxy needed for MVP.

### Q4: Secrets — Answered

**Decision:** 9p share for MVP. Worker writes secrets to a host directory, mounts as virtio-9p into the VM. Custom vault-like solution is a future goal.

### Q5: Resource Accounting — Open

The worker currently doesn't track resource usage. The VmSpec has `cpu` and `memoryBytes` fields. Worker should eventually sum allocated resources and reject creates that would exceed host capacity. Not needed for MVP.

### Q6: Result Collection & Reporting — Answered

**Decision for MVP:** No periodic push. Worker polls CH via `vm.info` (state) and `vm.counters` (I/O metrics). `listVms` returns this to the CLI. The CLI (or future control plane) is the poller.

Future: periodic push upstream, exit code capture, artifact extraction.

### Q7: Fault Injection Interface — New

For deterministic simulation testing, the worker (or a test harness) introduces failures into running VMs:
- **Network:** Drop packets, add latency, partition (via nftables on host)
- **Process:** Kill processes inside VM (via SSH or CH API)
- **Disk:** Corrupt files, fill disk (via 9p share manipulation or SSH)
- **Resources:** Reduce CPU/memory (CH supports hotplug)

**Open:** Should this be part of the worker's RPC API? Or a separate test harness that talks to CH directly? If the worker, we'd need capnp methods like `injectFault(vmId, faultSpec)`. If separate, the harness just needs the CH socket path.

For MVP: not needed. Focus on create/destroy. Fault injection is a future feature.

---

## Next Steps (Implementation Order — Worker-Only MVP)

The control plane, API gateway, and session orchestration are deferred. Focus is entirely on the worker and CLI.

### Phase 1: Worker RPC (create/destroy)
1. **worker.capnp:** Add `createVm @2 (spec :Common.VmSpec) -> (id :Text)` and `deleteVm @3 (id :Text) -> ()`
2. **server.rs:** Implement RPC handlers — translate to `CommandPayload::Create` / `CommandPayload::Delete`, send through `CommandSender`
3. **CLI (pcr-worker-test):** Add `create-vm` and `delete-vm` subcommands
   - `create-vm --spec-json '{"kernel":"/nix/store/...","disk":"/nix/store/...", ...}'` — CLI parses JSON, builds capnp VmSpec, sends createVm RPC
   - `create-vm --kernel <path> --disk <path> [--initrd <path>] ...` — individual args for quick tests
   - `delete-vm <id>` — sends deleteVm RPC
4. **End-to-end test:** CLI → Worker → CH process spawns → VM boots → CLI lists VMs → CLI deletes VM

### Phase 2: Image availability
5. **For MVP:** Worker assumes store paths are on the local Nix store (user built locally on same host). No `nix copy` needed yet.
6. **CLI input:** Two modes — `--spec-json` (full JSON blob from `nix eval`) or individual `--kernel/--disk/--initrd/--cmdline/--cpu/--memory` args
7. **Future:** Worker runs `nix copy --from <cache> <store-path>` before creating VM. Adds a binary cache URL to worker config.

### Phase 3: VM access & observability
7. **VM IP reporting:** Worker reports VM IP address in `listVms` response (from CH's `vm.info` or TAP device config)
8. **Serial console access:** Worker captures CH serial PTY output, makes it queryable (e.g., `get-logs <vm-id>` CLI command)
9. **SSH connectivity:** VMs have SSH enabled (already in vm-module.nix). CLI can show connection info: `ssh root@<vm-ip>`

### Phase 4: Secrets & network
10. **9p secrets mount:** Worker creates a per-VM host directory, writes secrets files, passes `--fs` to CH for 9p mount
11. **Network isolation:** Worker configures per-VM nftables rules from VmSpec's `allowedDomains` (already in host-module.nix, needs wiring)

### Future (post-MVP)
- Result collection & periodic reporting to upstream
- Fault injection API for deterministic simulation testing
- Resource accounting (reject overcommit)
- Control plane integration
- Webhook/trigger layer
- Custom secrets service
