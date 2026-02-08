{ lib, ... }:

with lib;

let
  memoryType = types.submodule {
    options = {
      amount = mkOption {
        type = types.float;
        default = 1.0;
        description = "Memory amount";
      };
      unit = mkOption {
        type = types.enum [ "MB" "GB" ];
        default = "GB";
        description = "Memory unit";
      };
    };
  };
in {
  options.cluster.vms = mkOption {
    type = types.attrsOf (types.submodule ({ name, config, ... }: {
      options = {
        cpu = mkOption {
          type = types.float;
          default = 1.0;
          description = "CPU cores";
        };

        memory = mkOption {
          type = memoryType;
          description = "Memory specification";
        };

        labels = mkOption {
          type = types.listOf types.str;
          default = [];
          description = "Labels for scheduling and filtering";
        };

        replicas = mkOption {
          type = types.uint;
          default = 1;
          description = "Number of replicas";
        };
      };
    }));
    description = "Declarative VM definitions";
    default = {};
  };

  config = {};
}
