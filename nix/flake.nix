{
  description = "Procurator an orchestrator framework for your cluster";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

  outputs = {
    self,
    nixpkgs,
  }: let
    lib = nixpkgs.lib;
    system = "x86_64-linux";
  in {
    # Export the cluster module for use in other flakes
    nixosModules.cluster = import ./modules/cluster.nix;

    # Expose a library function to evaluate a cluster config
    lib.evalCluster = { clusterConfig }:
      let
        eval = lib.evalModules {
          modules = [
            self.nixosModules.cluster
            clusterConfig
          ];
        };
      in {
        # Raw evaluated config (for control plane to read)
        config = eval.config.cluster;
        # Also expose errors/warnings if evaluation failed
        _module = eval._module;
      };

    # Note: nixosConfigurations would be built per-flake that uses procurator
    # This is just exposing the module and lib functions for downstream flakes
  };
}
