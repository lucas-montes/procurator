{
  pkgs,
  flake-utils,
  packages,
}:
let
  inherit (packages) worker cli;

  wokerAddr = "0.0.0.0:8080";

  mkAppWithDescription =
    drv: description:
    (flake-utils.lib.mkApp { inherit drv; })
    // {
      inherit description;
    };

  worker-wrapper =
    let
      configFile = pkgs.writeText "procurator-worker-config.json" (
        builtins.toJSON {
          listen_addr = wokerAddr;
          master_addr = "0.0.0.0:8081";
          health_tick_millis = 1000;
          vmm = {
            binary_path = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
            runtime_dir = "worker/tests/data";
            state_dir = "worker/tests/data";
            bridge_name = "br0";
            ip_pool_start = "10.0.0.2";
            ip_pool_end = "10.255.255.254";
            ip_netmask = "255.0.0.0";
          };
        }
      );
    in
    pkgs.writeShellScriptBin "procurator-worker" "
      echo 'We need the worker to be sudo so it can manage TAP devices'
      sudo ${worker}/bin/worker ${configFile}
    ";

  worker-test-wrapper = pkgs.writeShellScriptBin "procurator-worker-test" ''
    ${cli}/bin/pcr-worker-test --addr ${wokerAddr} "$@"
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
