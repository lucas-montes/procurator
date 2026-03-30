{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator;
in {
  # top-level options for the procurator bundle
  options.services.procurator = {
    enable = mkEnableOption "Enable procurator integration (aggregator)";

    systemUser = mkOption {
      type = types.str;
      default = "procurator";
      description = "System user for procurator services";
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [];
      description = "Packages added to the system profile when procurator is enabled";
    };
  };

  # import focused submodules (each declares services.procurator.<name> options)
  # Read more about https://nixos.org/manual/nixos/stable/index.html#sec-writing-modules
  imports = [
    ./worker
    ./cluster.nix
    ./control-plane.nix
    ./ci-service.nix
    ./cache.nix
    ./repohub
  ];

  config = mkIf cfg.enable {
    # create service user and data dir used by submodules
    users.users."${cfg.systemUser}" = {
      isSystemUser = true;
      createHome = false;
      description = "procurator service account";
      extraGroups = [ "network" ];
      shell = pkgs.runtimeShell;
    };

    environment.systemPackages = cfg.extraPackages;
    environment.etc."procurator".source = pkgs.lib.mkForce ./.; # optional: provide repo files to /etc/procurator

  };
}
