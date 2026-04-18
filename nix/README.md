# Nix — Infrastructure & VM Platform

## What

The Nix infrastructure layer for procurator. Builds all Rust binaries as Nix derivations, defines NixOS VM images, configures host networking, and provides the deployment modules users reference in their own flakes.

## Why

Nix is the foundational technology — it provides reproducible builds, immutable VM images, and declarative system configuration. This directory is where "Git commit" becomes "deployable VM image." Without it, there's no GitOps pipeline.


## See Also

- [GitOps Workflow Reference](GITOPS_WORKFLOW.md) — step-by-step: git push → running VM
- [Service Modules](modules/SERVICE_MODULES.md) — NixOS module usage for deploying procurator services
