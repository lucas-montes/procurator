{
  description = "Orchestrator framework";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";

  outputs = {
    self,
    nixpkgs,
  }: let
    lib = nixpkgs.lib;
  in {
    nixosModules.cluster = import ./modules/cluster.nix;

    lib.evalCluster = {clusterModule}: let
      eval = lib.evalModules {
        modules = [
          self.nixosModules.cluster
          clusterModule
        ];
      };
    in {
      desiredState = eval.config.cluster.vms;

      vmImages =
        lib.mapAttrs
        (name: _:
          nixpkgs.runCommand "vm-${name}" {} ''
            mkdir -p $out
            echo ${name} > $out/name
          '')
        eval.config.cluster.vms;
    };
  };
}
