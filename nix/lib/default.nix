# nix/lib/default.nix — Public API for procurator Nix libraries.
#
# Exports:
#   mkVmProfile  — Pure validation + normalization of VM guest config
#   mkVmImage    — Build NixOS disk image + vmSpec from a profile
#   mkVmSpecJson — Convenience: build image and return just the JSON spec
#   evalCluster  — Validate cluster topology with profile references
#
# Usage (from a consuming flake):
#   procurator.lib.${system}.mkVmProfile { ... }
#   procurator.lib.${system}.mkVmImage { profile = ...; }

{ pkgs, nixpkgs, system }:

let
  mkVmProfile = import ./profile {};
  mkVmImage = import ./image { inherit pkgs nixpkgs system; };
  evalCluster = import ./cluster {};
in {
  inherit mkVmProfile mkVmImage evalCluster;

  # Convenience: build a VM image and return just the JSON spec derivation.
  mkVmSpecJson = args: (mkVmImage args).vmSpecJson;
}
