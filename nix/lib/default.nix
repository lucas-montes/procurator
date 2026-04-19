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
in
{
  inherit diskVm workerLib;
}
