# nix/lib/default.nix — Public API for procurator Nix libraries.
#
# Exports:
#   mkVmProfile  — Pure validation + normalization of VM guest config
#   mkVmImage    — Build NixOS disk image + vmSpec from a profile
#   mkVmSpecJson — Convenience: build image and return just the JSON spec
#   evalCluster  — Validate cluster topology with profile references
#   mkSandbox    — Dockerfile-like API for building CH sandbox VMs
#
# Usage (from a consuming flake):
#   procurator.lib.${system}.mkVmProfile { ... }
#   procurator.lib.${system}.mkVmImage { profile = ...; }
#   procurator.lib.${system}.mkSandbox { entrypoint = "python3 /workspace/main.py"; ... }

{ pkgs, nixpkgs, system }:

let
  mkVmProfile = import ./profile {};
  mkVmImage = import ./image { inherit pkgs nixpkgs system; };
  mkSandbox = import ./sandbox { inherit pkgs nixpkgs system; };
  evalCluster = import ./cluster {};
in {
  inherit mkVmProfile mkVmImage mkSandbox evalCluster;

  # Convenience: build a VM image and return just the JSON spec derivation.
  mkVmSpecJson = args: (mkVmImage args).vmSpecJson;
}
