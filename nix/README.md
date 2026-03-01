# Nix — Infrastructure & VM Platform

## What

The Nix infrastructure layer for procurator. Builds all Rust binaries as Nix derivations, defines NixOS VM images, configures host networking, and provides the deployment modules users reference in their own flakes.

## Why

Nix is the foundational technology — it provides reproducible builds, immutable VM images, and declarative system configuration. This directory is where "Git commit" becomes "deployable VM image." Without it, there's no GitOps pipeline.

## Structure

```
nix/
├── flake.nix              # Entry point: exports packages, modules, libs, dev shells
├── lib/                   # 4-layer VM building pipeline
│   ├── default.nix        #   Entry point (mkVmProfile, mkVmImage, mkVmSpecJson, evalCluster)
│   ├── profile/           #   Layer 1: VM profiles (validation + normalization)
│   ├── image/             #   Layer 2: Profile → NixOS image + vmSpec
│   │   └── vm-module.nix  #     Guest NixOS module (systemd, SSH, virtio)
│   └── cluster/           #   Layer 3: Cluster topology validation
├── modules/               # NixOS service modules
│   ├── host/              #   Host networking (bridge, TAP, NAT, nftables, dnsmasq)
│   ├── procurator-worker.nix
│   ├── procurator-control-plane.nix
│   ├── ci-service.nix
│   ├── cache.nix
│   ├── repohub.nix
│   └── cluster.nix        #   Declarative VM topology
├── tests/                 # Fast (nix-instantiate) + integration (NixOS test)
│   ├── profile-fast.nix   #   19 assertions
│   ├── vm-spec-fast.nix   #   9 assertion groups
│   ├── cluster-fast.nix   #   10 assertion groups
│   └── integration/       #   Full VM build + boot test
└── flake-vmm/             # Legacy monolithic VM builder (being replaced by lib/)
```

## 4-Layer Lib Pipeline

```
Profile (mkVmProfile)          Declare what a VM should be
    │                          ↓ validates + normalizes
Image (mkVmImage)              Build NixOS image + vmSpec JSON
    │                          ↓ kernel, initrd, disk, cmdline
Cluster (evalCluster)          Validate topology (profiles × nodes)
    │                          ↓ all profiles resolve, no dangling refs
Host (NixOS module)            Configure the physical host networking
```

## GitOps Workflow Reference

- Store path: A path in store like /nix/store/abc123-nixos-system-25.11
- Closure: A store path + all its dependencies (transitive). When you "copy a closure" you copy the whole dependency tree.
- Binary cache: A server (cachix, attic, S3+nix-serve) that stores NARs (Nix ARchive = compressed store paths). Workers fetch from cache instead of building locally.

## GitOps workflow (steps + commands)
All steps are executed by your Rust CLI/CI logic. The commands below are **reference invocations** the Rust code should run (via `std::process::Command`).

### 0) Preconditions
- Flake config lives in `example/flake.nix` and uses the pinned `procurator` input
- CI/CD runs on every push
- Workers are bare‑metal hosts running NixOS with `nix` and `cloud-hypervisor`
- Control plane runs on a separate machine

### 1) User edits and pushes
Edit the flake and push to git:

```nu
git add example/flake.nix
git commit -m "Update cluster"
git push
```

### 2) CI/CD evaluates the blueprint
Generate a serializable desired state from the flake (source‑of‑truth config only):

```nu
nix eval --json "example#blueprintJSON" > blueprint.json
```

### 3) CI/CD builds system closures and detects changes
Build the NixOS system closures for each VM. If nothing changed, Nix reuses cached results. Then compare new store paths to the previous deployment artifact to decide whether to deploy.

```nu
for vm in control-plane-1 worker-1 worker-2 {
  let closure = (nix build --no-link --print-out-paths $"example#nixosConfigurations.($vm).config.system.build.toplevel")
  print $"($vm) => ($closure)"
}

# Change detection (CI): compare with previous deployment.json
# If any VM closure path differs, mark "changed = true" and proceed
# Otherwise, skip deployment notification
```

### 4) CI/CD publishes to a custom binary cache
Publish all built store paths so workers can pull without building. Use your own cache (attic or nix-serve).

```nu
# Example with attic (self‑hosted custom cache)
attic push your-cache:default /nix/store/abc123-nixos-system-25.11

# Example with nix-serve (custom cache via ssh-ng)
nix copy --to ssh-ng://cache-host /nix/store/abc123-nixos-system-25.11
```

### 5) CI/CD generates a deployment artifact
Produce a deployment payload that includes:
- `blueprint.json`
- store paths per VM (closures)
- git commit hash

Recommended: store as a CI artifact (not committed to git), e.g. `deployment.json`.

### 6) CI/CD notifies control plane
Only if changes are detected, notify the control plane and pass `deployment.json` (or its location).

### 7) Control plane decides what changed
Compare desired closure (from artifact) with the live system on each worker:

```nu
ssh root@192.168.1.11 readlink -f /run/current-system
```

If the store path differs, deploy; if identical, skip.

### 8) Control plane sends desired state to workers
Control plane sends per‑VM desired closure (store path) and metadata to the target worker.

### 9) Workers pull and activate
Workers pull from cache (or receive pushed closures) and activate:

```nu
# Pull closure from cache (if available)
nix copy --from https://your-cache.example.org /nix/store/abc123-nixos-system-25.11

# Activate (test -> health checks -> boot)
sudo /nix/store/abc123-nixos-system-25.11/bin/switch-to-configuration test
sudo /nix/store/abc123-nixos-system-25.11/bin/switch-to-configuration boot
```

### 10) Rollback on failure
If health checks fail, rollback to the previous generation:

```nu
sudo nixos-rebuild switch --rollback
```

## Where to store closures and metadata
- Common practice: **do not commit store paths or deployment artifacts to git**.
- Instead, **CI generates them per commit** and publishes:
	- a binary cache (NARs)
	- a deployment artifact (JSON) with the store paths + git hash
- `flake.lock` pins inputs, but **does not contain build outputs or store paths**.

## How to know if something changed (CI/CD)
- Build closures for all VMs.
- Load the previous `deployment.json` artifact from the last successful run.
- If any VM closure path differs, changes exist.
- If all paths match, skip notify/deploy.

## Should the cluster config keep closures?
- **No**. Keep the cluster config clean (desired intent only).
- Store **computed closures** in `deployment.json` produced by CI/CD.
- That artifact is the source of truth for deploy execution.

## Rust structs (minimal)
Use Rust to parse the JSON and orchestrate commands. Shapes below match `blueprintJSON` and `deployment.json`.

```json
{
	"gitCommit": "<hash>",
	"blueprint": {
		"worker-1": {
			"role": "worker",
			"cpu": 2.0,
			"memory": { "amount": 2.0, "unit": "GB" },
			"labels": ["worker", "compute"],
			"replicas": 1,
			"deployment": {
				"addr": "192.168.1.11",
				"backend": "cloud-hypervisor",
				"sshUser": "root",
				"sshPort": 22,
				"healthChecks": [ { "enabled": true, "command": "...", "timeout": 60, "interval": 10 } ],
				"autoRollback": true
			}
		}
	},
	"closures": {
		"worker-1": "/nix/store/abc123-nixos-system-25.11"
	}
}
```

Recommended Rust fields:
- `DeploymentPayload { git_commit, blueprint, closures }`
- `VmSpec { role, cpu, memory, labels, replicas, deployment }`
- `Deployment { addr, backend, ssh_user, ssh_port, health_checks, auto_rollback }`

## Recommended minimal payload (deployment.json)
- git commit hash
- blueprint JSON
- per‑VM system closure store paths
- optional kernel/initrd paths (can be derived from closure on workers)
