{
  pkgs,
  nixpkgs,
  system,
}:

let
  # export diskVm as a function that merges caller args with the library defaults
  diskVm = args: import ./diskVm.nix ({ inherit pkgs nixpkgs system; } // args);

  # worker config builder — shared between apps.nix (dev) and service.nix (NixOS module)
  workerLib = import ./worker.nix { inherit pkgs; };

  # a repo's `procuratorDeployment` output, read by the CI and deployed
  deployment = import ./deployment.nix { inherit pkgs; };
in
{
  inherit diskVm workerLib deployment;
}
