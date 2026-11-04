{lib, ...}:
with lib; let
  deploymentType = types.submodule {
    options = {
      addr = mkOption {
        type = types.str;
        description = "IP address or hostname of the VM";
        example = "192.168.1.10";
      };
    };
  };
in {
  # Named VM registry consumed by services.procurator.control-plane (peers/workers)
  # and services.procurator.worker (master) to resolve addresses/labels by name
  # instead of hardcoding them. Only `labels` and `deployment.addr` are read by
  # those modules today.
  options.cluster = {
    vms = mkOption {
      type = types.attrsOf (types.submodule ({
        name,
        config,
        ...
      }: {
        options = {
          role = mkOption {
            type = types.enum ["control-plane" "worker"];
            default = "worker";
            description = "Role of this VM in the cluster";
          };

          labels = mkOption {
            type = types.listOf types.str;
            default = [];
            description = "Labels for scheduling and filtering";
          };

          deployment = mkOption {
            type = deploymentType;
            description = "Deployment configuration";
          };
        };
      }));
      description = "Declarative VM registry (name -> role/labels/address)";
      default = {};
    };
  };

  config = {};
}
