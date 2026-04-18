{
  pkgs,
  nixpkgs,
  system,
}:

let
  # export diskVm as a function that merges caller args with the library defaults
  diskVm = args: import ./diskVm.nix ({ inherit pkgs nixpkgs system; } // args);
in
{
  inherit diskVm;
}
