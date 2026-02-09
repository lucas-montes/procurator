{lib, ...}:
with lib; let
  memoryType = types.submodule {
    options = {
      amount = mkOption {
        type = types.float;
        default = 1.0;
        description = "Memory amount";
      };
      unit = mkOption {
        type = types.enum ["MB" "GB"];
        default = "GB";
        description = "Memory unit";
      };
    };
  };

  healthCheckType = types.submodule {
    options = {
      enabled = mkOption {
        type = types.bool;
        default = true;
        description = "Enable health check";
      };
      command = mkOption {
        type = types.str;
        description = "Command to run for health check";
        example = "systemctl is-active nginx";
      };
      timeout = mkOption {
        type = types.int;
        default = 30;
        description = "Health check timeout in seconds";
      };
      interval = mkOption {
        type = types.int;
        default = 10;
        description = "Health check interval in seconds";
      };
    };
  };

  deploymentType = types.submodule {
    options = {
      addr = mkOption {
        type = types.str;
        description = "IP address or hostname of the VM";
        example = "192.168.1.10";
      };
      backend = mkOption {
        type = types.enum ["cloud-hypervisor" "libvirt" "qemu"];
        default = "cloud-hypervisor";
        description = "Hypervisor backend";
      };
      sshUser = mkOption {
        type = types.str;
        default = "root";
        description = "SSH user for deployment";
      };
      sshPort = mkOption {
        type = types.port;
        default = 22;
        description = "SSH port for deployment";
      };
      healthChecks = mkOption {
        type = types.listOf healthCheckType;
        default = [];
        description = "List of health checks to run after activation";
      };
      autoRollback = mkOption {
        type = types.bool;
        default = true;
        description = "Automatically rollback on health check failure";
      };
    };
  };
in {
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
            type = types.int;
            default = 1;
            description = "Number of replicas";
          };

          deployment = mkOption {
            type = deploymentType;
            description = "Deployment configuration";
          };

          # TODO: change this to be somehting that would build a smaller image liek alpine or so
          configuration = mkOption {
            type = types.deferredModule;
            default = {};
            description = "NixOS module configuration for this VM";
          };
        };
      }));
      description = "Declarative VM definitions";
      default = {};
    };
  };

  config = {};
}
