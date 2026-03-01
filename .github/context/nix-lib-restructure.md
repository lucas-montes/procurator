# Nix Library Restructuring — Design Document

## Problem Statement

The current Nix code mixes concerns across `nix/flake-vmm/` and `nix/modules/` without clear boundaries:

1. **`flake-vmm/flake.nix`** does too much: kernel selection, NixOS image building, VM spec generation, vmSpecJson, host networking, CH launching, workload injection — all in one 800-line file.
2. **`nix/modules/cluster.nix`** defines VM topology but has no connection to the image builder. Its `configuration = deferredModule` is a raw NixOS module that doesn't use `mkVmImage`.
3. **No "Lego" composability** — you can't mix and match cluster topology + VM config + kernel choice independently.

## Architecture: Four Layers

```
                    User's flake.nix
                          │
            ┌─────────────┼─────────────┐
            ▼             ▼             ▼
       ┌─────────┐  ┌──────────┐  ┌─────────┐
       │ cluster │  │ profile  │  │  host   │
       │         │  │          │  │         │
       │ topology│  │ guest cfg│  │ services│
       │ roles   │  │ packages │  │ network │
       │ deploy  │  │ workload │  │ firewall│
       └────┬────┘  └────┬─────┘  └─────────┘
            │             │
            │    ┌────────▼────────┐
            │    │     image       │
            │    │                 │
            │    │ kernel + rootfs │
            │    │ disk image      │
            │    │ vmSpec JSON     │
            │    └────────┬────────┘
            │             │
            ▼             ▼
       Control Plane    Worker
       (scheduling)     (boot VM)
```

### Layer 1: Profile — "What goes inside a VM + how big it is"

`mkVmProfile` validates and normalizes guest configuration into a pure attrset.
No NixOS eval, no image building — just data.

**Fields:**
- `hostname` (str, default: "ch-vm")
- `cpu` (int, default: 1) — single source of truth for resources
- `memoryMb` (int, default: 512) — single source of truth for resources
- `packages` (pkgs -> [package], default: (_: []))
- `entrypoint` (str or null, default: null)
- `autoShutdown` (bool, default: false)
- `allowedDomains` ([str], default: [])
- `sshAuthorizedKeys` ([str], default: [])
- `files` ({path = content}, default: {})

### Layer 2: Image — "How to produce the artifact"

`mkVmImage` takes a profile + image-build options, produces `{ image, vmSpec, vmSpecJson, nixos }`.

**Arguments:**
- `profile` (required — from mkVmProfile)
- `kernel` (package, default: pkgs.linux_6_6)
- `diskSize` (str, default: "auto")
- `additionalSpace` (str, default: "256M")

**Output:**
- `image` — ext4 raw disk image derivation
- `nixos` — full NixOS system (for kernel/initrd access)
- `toplevel` — system.build.toplevel
- `vmSpec` — attrset matching 8-field capnp VmSpec
- `vmSpecJson` — derivation producing vm-spec.json

### Layer 3: Cluster — "What should exist"

`evalCluster` validates topology, links profiles to VMs for scheduling.

**Key change:** Drop `configuration = deferredModule`, use `vmProfile` reference instead.
Resources come from the profile — no duplication.

### Layer 4: Host — "How to run VMs on a machine"

NixOS modules for the physical server: bridge networking, TAP devices, NAT,
dnsmasq, per-VM nftables domain allowlisting, procurator daemon systemd units.

## Data Flow

```
mkVmProfile { cpu, memoryMb, packages, entrypoint, allowedDomains, ... }
      │
      ├──→ mkVmImage { profile }
      │       ├── reads packages, entrypoint, files → vm-module.nix (guest config)
      │       ├── reads cpu, memoryMb → vmSpec fields
      │       └── produces: image, vmSpec, vmSpecJson
      │
      └──→ evalCluster { vms.X.vmProfile = profile }
              ├── reads cpu, memoryMb → scheduling decisions
              ├── reads deployment.addr → where to run
              └── produces: validated cluster topology
```

Kernel/disk options are NOT in the profile — they're image-build concerns:
```nix
vm = mkVmImage { profile = sandbox; };                           # default kernel
vm = mkVmImage { profile = sandbox; kernel = pkgs.linux_6_12; }; # custom kernel
```

## `allowedDomains` Enforcement Model

Enforcement is **host-side only** (nftables on the VM's TAP device):

```
VM profile: allowedDomains = [ "api.openai.com" "github.com" ]
    │
    ▼ (carried through)
vmSpec JSON → capnp VmSpec → createVm RPC → worker
    │
    ▼ (host reads it)
Host NixOS module: per-VM nftables rules on the TAP device
  - Always allow DNS (udp/tcp 53)
  - Resolve allowedDomains → IP sets
  - Allow only those IPs
  - Drop everything else from this VM's TAP
    │
    ▼
Guest cannot bypass — host controls the network pipe
```

## Composition Examples

**Simple (LLM sandbox — profile + image, no cluster):**
```nix
profile = procurator.lib.${system}.mkVmProfile {
  hostname = "sandbox";
  packages = p: [ p.python3 ];
  cpu = 2;
  memoryMb = 1024;
  allowedDomains = [ "api.openai.com" ];
};

vm = procurator.lib.${system}.mkVmImage { profile = profile; };
# Output: vm.image, vm.vmSpec, vm.vmSpecJson
```

**Workload VM (Python script as entrypoint):**
```nix
profile = procurator.lib.${system}.mkVmProfile {
  hostname = "worker";
  packages = p: [ p.python3 myApp ];
  entrypoint = "${myApp}/bin/${myApp.meta.mainProgram}";
  autoShutdown = true;
  allowedDomains = [ "pypi.org" ];
  cpu = 2;
  memoryMb = 1024;
};

vm = procurator.lib.${system}.mkVmImage { profile = profile; };
```

**Full cluster:**
```nix
profiles = {
  sandbox = procurator.lib.${system}.mkVmProfile {
    hostname = "sandbox";
    cpu = 4; memoryMb = 2048;
    packages = p: [ p.python3 p.git ];
    allowedDomains = [ "api.openai.com" "github.com" ];
  };
  ci = procurator.lib.${system}.mkVmProfile {
    hostname = "ci-runner";
    cpu = 2; memoryMb = 1024;
    packages = p: [ p.nix p.git p.cachix ];
    entrypoint = "${ciScript}/bin/run-ci";
    autoShutdown = true;
  };
};

cluster = procurator.lib.${system}.evalCluster {
  vms = {
    sandbox-1 = { vmProfile = profiles.sandbox; deployment.addr = "10.0.0.1"; };
    sandbox-2 = { vmProfile = profiles.sandbox; deployment.addr = "10.0.0.2"; };
    ci-runner  = { vmProfile = profiles.ci; deployment.addr = "10.0.0.3"; };
  };
};

# Build images for all VMs in the cluster
images = lib.mapAttrs (name: vm:
  procurator.lib.${system}.mkVmImage { profile = vm.vmProfile; }
) cluster.vms;
```

**Host NixOS config:**
```nix
imports = [ procurator.nixosModules.host ];

procurator.host = {
  enable = true;
  worker = { enable = true; masterAddr = "10.0.0.1:8080"; };
  vms = {
    sandbox-1.allowedDomains = [ "api.openai.com" "github.com" ];
    ci-runner = {}; # full internet
  };
};
```

## Key Decisions

- **Resources in profile only.** `cpu` and `memoryMb` live in the profile. No override chains, no duplication.
- **No `mkVmFromDrv`.** No sugar. Users write explicit profiles with `packages` + `entrypoint`.
- **No inline params to `mkVmImage`.** Always requires an explicit `profile` argument.
- **No overrides.** Profile is the single source of truth.
- **Tests, not examples.** Fast pure-eval tests per lib + slow integration test with `test.py`.

## File Layout

```
nix/
├── flake.nix                          # Top-level: Rust crates + dev shell + exports libs
│
├── lib/
│   ├── default.nix                    # Entry point: exports mkVmProfile, mkVmImage, mkVmSpecJson
│   │
│   ├── profile/
│   │   └── default.nix                # mkVmProfile: validates + normalizes guest config
│   │
│   ├── image/
│   │   ├── default.nix                # mkVmImage: profile + kernel opts → { image, vmSpec, vmSpecJson }
│   │   └── vm-module.nix              # NixOS guest module (systemd, SSH, virtio, workload)
│   │
│   └── cluster/
│       └── default.nix                # evalCluster: validates topology, links profiles to VMs
│
├── modules/
│   ├── host/
│   │   ├── default.nix                # Combined host NixOS module
│   │   ├── networking.nix             # Bridge, TAP, NAT, dnsmasq
│   │   └── firewall.nix              # Per-VM nftables domain allowlisting
│   │
│   ├── procurator-worker.nix          # Worker daemon systemd service (unchanged)
│   ├── procurator-control-plane.nix   # Control plane systemd service (unchanged)
│   ├── cache.nix                      # Binary cache service (unchanged)
│   ├── ci-service.nix                 # CI service (unchanged)
│   └── repohub.nix                    # Repohub service (unchanged)
│
├── tests/
│   ├── profile-fast.nix               # Pure eval: mkVmProfile shape, defaults, validation
│   ├── vm-spec-fast.nix               # Pure eval: vmSpec 8-field contract, JSON round-trip
│   ├── cluster-fast.nix               # Pure eval: evalCluster validation
│   └── integration/
│       ├── flake.nix                  # Slow test flake: builds real VM with test.py
│       └── test.py                    # Python workload for integration test
│
└── README.md
```

## Lib Entry Point

```nix
# nix/lib/default.nix
{ pkgs, nixpkgs, system }: {
  mkVmProfile = import ./profile { inherit pkgs; };
  mkVmImage = import ./image { inherit pkgs nixpkgs system; };
  mkVmSpecJson = args: (import ./image { inherit pkgs nixpkgs system; } args).vmSpecJson;
  evalCluster = import ./cluster { inherit pkgs; };
}
```
