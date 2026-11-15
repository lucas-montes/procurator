{ config, lib, ... }:

with lib;

let
  cfg = config.services.procurator.journalGateway;
in {
  # Lets the read-only Procurator console follow this host's service logs
  # (procurator-control-plane / procurator-worker units) over HTTP. Nothing is
  # pushed: the console pulls `/entries?follow&_SYSTEMD_UNIT=<unit>`.
  #
  # journal-gatewayd has no authentication: anyone who can reach the port can
  # read the whole system journal. Keep it on a trusted network.
  options.services.procurator.journalGateway = {
    enable = mkEnableOption "systemd-journal-gatewayd so the Procurator console can read this host's logs";

    port = mkOption {
      type = types.port;
      default = 19531;
      description = "Port journal-gatewayd listens on.";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open the gateway port in the firewall (unauthenticated: trusted networks only).";
    };
  };

  config = mkIf cfg.enable {
    services.journald.gateway = {
      enable = true;
      port = cfg.port;
    };

    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.port ];
  };
}
