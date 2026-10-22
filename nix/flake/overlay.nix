# flake.output "overlays.default": adds every Procurator Rust package to
# `pkgs` (pkgs.cache, pkgs.ci_service, pkgs.worker, pkgs.cli,
# pkgs.control_plane, pkgs.repohub) so the NixOS modules' package defaults
# resolve for consumers of `nixosModules.procurator` without extra wiring:
#
#   overlays = [ procurator.overlays.default ];
{ pkgs, src, naersk ? null }:
let
  workspaceRoot = import ./workspaceRoot.nix { inherit pkgs src; };
in
import ./packages.nix { inherit pkgs workspaceRoot naersk; }
