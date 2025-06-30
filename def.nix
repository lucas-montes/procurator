let
  pkgs = import <nixpkgs> {system = "x86_64-linux";}; # Adjust system if needed
  stateLib = import ./state-lib.nix {inherit pkgs;};
  myConfig = stateLib.defaultConfig; # Use default or provide your own
in {
  state = stateLib.generateStateLock {config = myConfig;};
  stateCheck = stateLib.validateStateLock {stateLock = stateLib.generateStateLock {config = myConfig;};};
}
