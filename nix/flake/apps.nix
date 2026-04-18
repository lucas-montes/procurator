{
  pkgs,
  flake-utils,
  packages,
}:
let
  inherit (packages) worker;

  worker-wrapper =
    let
      configFile = pkgs.writeText "procurator-worker-config.json" (
        builtins.toJSON {
          listen_addr = "0.0.0.0:8080";
          master_addr = "0.0.0.0:8081";
          health_tick_millis = 1000;
          vmm = {
            binary_path = "${pkgs.cloud-hypervisor}/bin/cloud-hypervisor";
            socket_dir = "/run/procurator-worker/vms";
            socket_timeout_secs = 10;
            bridge_name = "bro0";
          };
        }
      );
    in
    pkgs.writeShellScriptBin "procurator-worker" "
      ${worker}/bin/worker ${configFile}
    ";
in
{

  apps = {
    worker = flake-utils.lib.mkApp { drv = worker-wrapper; };
  };
}
