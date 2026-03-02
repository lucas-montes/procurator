{
  pkgs,
  flake-utils,
  packages,
}: let
  inherit (packages)
    cache
    ci_service
    worker
    cli
    ;

  worker-wrapper = pkgs.writeShellScriptBin "procurator-worker" ''
    exec ${worker}/bin/worker "$@"
  '';

  control-plane-wrapper = pkgs.writeShellScriptBin "procurator-control-plane" ''
    exec ${worker}/bin/worker "$@"
  '';

  pcr-test-wrapper = pkgs.writeShellScriptBin "pcr-test" ''
    exec ${cli}/bin/pcr-test "$@"
  '';
in {
  wrappers = {
    inherit
      worker-wrapper
      control-plane-wrapper
      pcr-test-wrapper
      ;
  };

  apps = {
    cache = flake-utils.lib.mkApp {drv = cache;};
    ci_service = flake-utils.lib.mkApp {drv = ci_service;};
    worker = flake-utils.lib.mkApp {drv = worker;};
    pcr-test = flake-utils.lib.mkApp {
      drv = cli;
      exePath = "/bin/pcr-test";
    };
    procurator-worker = flake-utils.lib.mkApp {drv = worker-wrapper;};
    procurator-control-plane = flake-utils.lib.mkApp {drv = control-plane-wrapper;};
    default = flake-utils.lib.mkApp {drv = control-plane-wrapper;};
  };
}
