{
  pkgs,
  flake-utils,
  packages,
  workerLib,
}:
let
  inherit (packages) worker cli;
  inherit (workerLib) mkWorkerConfig defaults;

  workerAddr = defaults.listenAddr;

  mkAppWithDescription =
    drv: description:
    (flake-utils.lib.mkApp { inherit drv; })
    // {
      inherit description;
    };

  worker-wrapper =
    let
      configFile = pkgs.writeText "procurator-worker-config.json" (
        builtins.toJSON (mkWorkerConfig {
          # Override only what differs from defaults for local dev.
          vmm = {
            runtimeDir = "worker/tests/data";
            stateDir   = "worker/tests/data";
          };
        })
      );
    in
    pkgs.writeShellScriptBin "procurator-worker" ''
      echo 'We need the worker to be sudo so it can manage TAP devices'
      sudo ${worker}/bin/worker ${configFile}
    '';

  worker-test-wrapper = pkgs.writeShellScriptBin "procurator-worker-test" ''
    ${cli}/bin/pcr-worker-test --addr ${workerAddr} "$@"
  '';
in
{

  apps = {
    worker = mkAppWithDescription worker-wrapper "Run the Procurator worker daemon";
    worker-test = mkAppWithDescription
      worker-test-wrapper
      "Run the test-only worker RPC CLI (read/list/create/delete)";
  };
}
