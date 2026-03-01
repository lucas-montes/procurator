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
└── GITOPS_WORKFLOW.md     # GitOps workflow reference (steps + commands)
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

## See Also

- [GitOps Workflow Reference](GITOPS_WORKFLOW.md) — step-by-step: git push → running VM
- [Service Modules](modules/SERVICE_MODULES.md) — NixOS module usage for deploying procurator services
