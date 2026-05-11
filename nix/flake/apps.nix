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

  # Local-dev TLS material. Generated on first run by the wrapper below;
  # the worker config points at the same path so they cannot drift.
  devTlsDir      = "worker/tests/data/dev-tls";
  devTlsCertPath = "${devTlsDir}/server.crt";
  devTlsKeyPath  = "${devTlsDir}/server.key";

  mkAppWithDescription =
    drv: description:
    (flake-utils.lib.mkApp { inherit drv; })
    // {
      inherit description;
    };

  configFile = pkgs.writeText "procurator-worker-config.json" (
    builtins.toJSON (mkWorkerConfig {
      vmm = {
        runtimeDir = "worker/tests/data";
        stateDir   = "worker/tests/data";
      };
      proxy = {
        tlsCertPath = devTlsCertPath;
        tlsKeyPath  = devTlsKeyPath;
      };
    })
  );

  worker-wrapper = pkgs.writeShellScriptBin "procurator-worker" ''
    set -euo pipefail

    if [ ! -s "${devTlsCertPath}" ] || [ ! -s "${devTlsKeyPath}" ]; then
      echo "[dev] Generating self-signed TLS cert at ${devTlsDir}"
      mkdir -p "${devTlsDir}"
      ${pkgs.openssl}/bin/openssl req -x509 -newkey rsa:2048 -nodes \
        -keyout "${devTlsKeyPath}" \
        -out    "${devTlsCertPath}" \
        -days 30 -subj "/CN=worker.local" 2>/dev/null
    fi

    echo "[dev] sudo is required so the worker can manage TAP devices."
    exec sudo ${worker}/bin/worker ${configFile}
  '';

  worker-test-wrapper = pkgs.writeShellScriptBin "procurator-worker-test" ''
    exec ${cli}/bin/pcr-worker-test --addr ${workerAddr} "$@"
  '';
in
{
  apps = {
    worker = mkAppWithDescription worker-wrapper
      "Run the Procurator worker daemon (auto-generates dev TLS cert)";
    worker-test = mkAppWithDescription worker-test-wrapper
      "Run the test-only worker RPC CLI (read/list/create/delete)";
  };
}
