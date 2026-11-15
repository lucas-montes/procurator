{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.console;
  clusterCfg = config.cluster.vms or {};

  # `deployment.addr` is an IP or hostname, sometimes with a port.
  hostOf = vm: head (splitString ":" clusterCfg.${vm}.deployment.addr);
  gatewayUrl = vm: "http://${hostOf vm}:${toString cfg.gatewayPort}";

  # Source objects match console::config::SourceConfig.
  gatewaySource = name: service: vm: unit: {
    inherit name service;
    kind.journal_gateway = {
      url = gatewayUrl vm;
      inherit unit;
    };
  };
  localSource = name: service: unit: {
    inherit name service;
    kind.journal_local = { inherit unit; };
  };

  sources =
    optional (cfg.controlPlane != null)
      (gatewaySource "control-plane" "control_plane" cfg.controlPlane "procurator-control-plane.service")
    ++ map (vm: gatewaySource vm "worker:${vm}" vm "procurator-worker.service") cfg.workers
    ++ optional cfg.local.controlPlane
      (localSource "control-plane" "control_plane" "procurator-control-plane.service")
    ++ map (id: localSource id "worker:${id}" "procurator-worker.service") cfg.local.workers
    ++ cfg.extraSources;

  configFile = pkgs.writeText "procurator-console-config.json" (builtins.toJSON {
    http_addr = cfg.httpAddr;
    bootstrap_entries = cfg.bootstrapEntries;
    stale_after_millis = cfg.staleAfterMillis;
    inherit sources;
  });
in {
  # Read-only web console: shows machines, VMs and the messages between
  # services live, built only from each service's own logs (see
  # context/specs/log-events.md). Remote hosts need
  # `services.procurator.journalGateway.enable = true`.
  options.services.procurator.console = {
    enable = mkEnableOption "Procurator read-only web console";

    package = mkOption {
      type = types.package;
      default = pkgs.console;
      defaultText = literalExpression "pkgs.console";
      description = "The console package to use.";
    };

    httpAddr = mkOption {
      type = types.str;
      default = "127.0.0.1:8095";
      description = "Address the web UI listens on. It has no authentication: keep it on loopback or a trusted network.";
    };

    controlPlane = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "cp-1";
      description = "Name in `cluster.vms` of the control-plane host, read through its journal gateway.";
    };

    workers = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ "worker-1" "worker-2" ];
      description = "Names in `cluster.vms` of worker hosts (the name is the worker id), read through their journal gateways.";
    };

    local = {
      controlPlane = mkOption {
        type = types.bool;
        default = false;
        description = "Read the control plane's journal on this host (`journalctl`) instead of through a gateway.";
      };

      workers = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Ids of workers running on this host, read with `journalctl`.";
      };
    };

    extraSources = mkOption {
      type = types.listOf types.attrs;
      default = [];
      example = literalExpression ''
        [ { name = "cp"; service = "control_plane"; kind.file.path = "/var/log/procurator/cp.log"; } ]
      '';
      description = "Additional raw source entries (see console/src/config.rs), e.g. log files.";
    };

    gatewayPort = mkOption {
      type = types.port;
      default = 19531;
      description = "journal-gatewayd port on the watched hosts.";
    };

    bootstrapEntries = mkOption {
      type = types.ints.positive;
      default = 2000;
      description = "Recent lines read back per source at startup to rebuild the current state.";
    };

    staleAfterMillis = mkOption {
      type = types.ints.positive;
      default = 15000;
      description = "A worker without a state line for this long is shown as unhealthy.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = all (vm: builtins.hasAttr vm clusterCfg) (cfg.workers ++ optional (cfg.controlPlane != null) cfg.controlPlane);
        message = "services.procurator.console: `controlPlane`/`workers` contain names not found in `cluster.vms`.";
      }
      {
        assertion = sources != [];
        message = "services.procurator.console: no log sources configured (set controlPlane, workers, local.* or extraSources).";
      }
    ];

    systemd.services.procurator-console = {
      description = "Procurator Console (read-only web UI)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # `journalctl` for local sources.
      path = [ pkgs.systemd ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/console ${configFile}";
        Restart = "on-failure";
        RestartSec = "5s";

        DynamicUser = true;
        # Read access to the local journal.
        SupplementaryGroups = [ "systemd-journal" ];
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        # `extraSources` files may live under /home.
        ProtectHome = "read-only";
      };
    };
  };
}
