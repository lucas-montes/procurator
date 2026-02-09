{
  description = "Example of flake to configure a cluster";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    procurator.url = "git+file:///home/lucas/Projects/procurator?dir=nix";
  };

  outputs = {
    self,
    nixpkgs,
    procurator,
  }: let
    lib = nixpkgs.lib;
    system = "x86_64-linux";
  in {
    # Evaluate the cluster configuration using procurator module
    blueprint = procurator.lib.evalCluster {
      clusterConfig = {
        cluster.vms = {
          # Control plane VM
          control-plane-1 = {
            role = "control-plane";
            cpu = 4.0;
            memory = {
              amount = 4.0;
              unit = "GB";
            };
            labels = ["control-plane"];
            replicas = 1;
            deployment = {
              addr = "192.168.1.10";
              backend = "cloud-hypervisor";
              sshUser = "root";
              sshPort = 22;
              healthChecks = [
                {
                  enabled = true;
                  command = "systemctl is-system-running --wait";
                  timeout = 60;
                  interval = 10;
                }
              ];
              autoRollback = true;
            };
            configuration = { lib, pkgs, ... }: {
              system.stateVersion = "25.11";
              services.openssh.enable = true;
              environment.systemPackages = with pkgs; [
                curl
                git
                htop
              ];
            };
          };

          # Worker VMs
          worker-1 = {
            role = "worker";
            cpu = 2.0;
            memory = {
              amount = 2.0;
              unit = "GB";
            };
            labels = ["worker" "compute"];
            replicas = 1;
            deployment = {
              addr = "192.168.1.11";
              backend = "cloud-hypervisor";
              sshUser = "root";
              sshPort = 22;
              healthChecks = [
                {
                  enabled = true;
                  command = "systemctl is-system-running --wait";
                  timeout = 60;
                  interval = 10;
                }
                {
                  enabled = true;
                  command = "test -S /run/cloud-hypervisor.sock";
                  timeout = 10;
                  interval = 5;
                }
              ];
              autoRollback = true;
            };
            configuration = { lib, pkgs, ... }: {
              system.stateVersion = "25.11";
              services.openssh.enable = true;
              environment.systemPackages = with pkgs; [
                curl
                git
                htop
              ];
            };
          };

          worker-2 = {
            role = "worker";
            cpu = 2.0;
            memory = {
              amount = 2.0;
              unit = "GB";
            };
            labels = ["worker" "compute"];
            replicas = 1;
            deployment = {
              addr = "192.168.1.12";
              backend = "cloud-hypervisor";
              sshUser = "root";
              sshPort = 22;
              healthChecks = [
                {
                  enabled = true;
                  command = "systemctl is-system-running --wait";
                  timeout = 60;
                  interval = 10;
                }
              ];
              autoRollback = true;
            };
            configuration = { lib, pkgs, ... }: {
              system.stateVersion = "25.11";
              services.openssh.enable = true;
              environment.systemPackages = with pkgs; [
                curl
                git
                htop
              ];
            };
          };
        };
      };
    };

    # Helper to build NixOS configurations for each VM
    nixosConfigurations = let
      vms = self.blueprint.config.vms or {};
    in lib.mapAttrs
      (name: vmConfig: lib.nixosSystem {
        inherit system;
        modules = [
          vmConfig.configuration
          {
            networking.hostName = name;
            # Expose metadata as environment variables for runtime access
            environment.variables.PROCURATOR_VM_NAME = name;
            environment.variables.PROCURATOR_VM_ROLE = vmConfig.role;
          }
        ];
      })
      vms;

    # Export only the serializable parts for control plane (excludes functions)
    blueprintJSON =
      let
        vms = self.blueprint.config.vms or {};
        # Filter out non-serializable fields (like configuration function)
        serializableVms = lib.mapAttrs
          (name: vmConfig: {
            role = vmConfig.role;
            cpu = vmConfig.cpu;
            memory = vmConfig.memory;
            labels = vmConfig.labels;
            replicas = vmConfig.replicas;
            deployment = vmConfig.deployment;
            # Note: configuration is omitted because it's a function
          })
          vms;
      in serializableVms;
  };
}
