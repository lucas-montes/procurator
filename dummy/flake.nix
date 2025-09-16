{
  description = "This flake will simulate a simple monorepo where infra and code are in the same repo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
    # Here i can use a relative path, but in prod i could use something better, same for the package in the services, how to handle those files?
    infrastructure = {
      url = "./infrastructure.nix";
      flake = false;
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    infrastructure,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
        };

        services = {
          # Define new services that point to a custom package
          dummy = {
            production = {
              cpu = 1.5;
              memory = {
                amount = 1;
                unit = "GB";
              };
              packages = self.packages.${system}.default;
            };
            staging = [
              {
                cpu = 1.1;
                memory = {
                  amount = 1;
                  unit = "GB";
                };
                packages = self.packages.${system}.default;
              }
            ];
          };
        };

        buildDummy = pkgs.stdenv.mkDerivation {
          pname = "dummy";
          version = "0.1.0";
          src = ./.;
          buildInputs = [pkgs.gcc pkgs.bash];
          doCheck = false; # Tests only in checks
          checkPhase = ''
            if [ -f ./test_dummy.sh ]; then
              ${pkgs.bash}/bin/bash ./test_dummy.sh
            else
              echo "Warning: test_dummy.sh not found"
              true
            fi
          '';
          buildPhase = ''
            if [ ! -f main.c ]; then
              echo "Error: main.c not found"
              exit 1
            fi
            gcc -o dummy main.c
          '';
          installPhase = ''
            mkdir -p $out/bin
            cp dummy $out/bin/
          '';
        };

        # VM configuration
        mkVM = env:
          nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              ({
                config,
                pkgs,
                ...
              }: {
                system.stateVersion = 24.11;
                virtualisation.vmVariant.virtualisation.diskSize = 2048;
                # virtualisation = {
                #   vmVariant.enable = true;
                #   diskSize = 2048; # 2GB disk
                #   memorySize = services.dummy.production.memory.amount * 1024;
                #   cores = services.dummy.production.cpu;# TODO: why it doesnt work? they are in the docs
                # };
                # boot.loader.systemd-boot.enable = true;
                # fileSystems."/" = {
                #   device = "/dev/disk/by-label/nixos";
                #   fsType = "ext4";
                #   autoFormat = true;
                # };
                networking.hostName = "dummy-vm-${env}";
                services.openssh = {
                  enable = true;
                  settings.PermitRootLogin = "prohibit-password";
                };
                # services.qemuGuest.enable = true;
                networking.firewall.allowedTCPPorts = [22];
                users.users.root.openssh.authorizedKeys.keys = [
                  # Replace with your SSH public key
                  "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI..."
                ];
                environment.systemPackages = [services.dummy.${env}.packages];
                systemd.services.dummy = {
                  description = "Dummy App Service";
                  wantedBy = ["multi-user.target"];
                  serviceConfig = {
                    ExecStart = "${self.packages.${system}.default}/bin/dummy";
                    Restart = "always";
                    CPUQuota = "${toString (services.dummy.production.cpu * 100)}%";
                    MemoryMax = "${toString services.dummy.production.memory.amount}${services.dummy.production.memory.unit}";
                  };
                };
              })
            ];
          };

        # Container host configuration
        containerHost = nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            ({
              config,
              pkgs,
              ...
            }: {
              boot.enableContainers = true;
              containers = {
                dummy-production = {
                  autoStart = true;
                  privateNetwork = true;
                  hostAddress = "192.168.100.10";
                  localAddress = "192.168.100.11";
                  config = {
                    config,
                    pkgs,
                    ...
                  }: {
                    networking.hostName = "dummy-container-production";
                    environment.systemPackages = [services.dummy.production.packages];
                    systemd.services.dummy = {
                      description = "Dummy App Service";
                      wantedBy = ["multi-user.target"];
                      serviceConfig = {
                        ExecStart = "${self.packages.${system}.default}/bin/dummy";
                        Restart = "always";
                        CPUQuota = "${toString (services.dummy.production.cpu * 100)}%";
                        MemoryMax = "${toString services.dummy.production.memory.amount}${services.dummy.production.memory.unit}";
                      };
                    };
                  };
                };
              };
            })
          ];
        };

        test-vm = nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            ./configuration.nix
          ];
        };
      in {
        packages = {
          # This package allow us to run build and have the state generated. Probably shouldn't be here?
          state = pkgs.writeTextFile {
            name = "state-lock";
            text = builtins.toJSON {
              inherit infrastructure services;
            };
            destination = "/state.json";
          };

          default = buildDummy;
        };

        hydraJobs = {
          inherit (self) packages; # Builds all packages
        };

        checks = {
          dummy-test =
            pkgs.runCommand "dummy-test" {
              buildInputs = [buildDummy pkgs.bash];
            } ''
              if [ -f ${./test_dummy.sh} ]; then
                ${pkgs.bash}/bin/bash ${./test_dummy.sh} > $out
              else
                echo "Warning: test_dummy.sh not found"
                true > $out
              fi
            '';
        };

        nixosConfigurations = {
          vm = mkVM "production";
          container-host = containerHost;
        };

        apps = {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/dummy";
            meta = with pkgs.lib; {
              description = "A dummy C executable for testing";
              license = licenses.mit;
            };
          };
          vm = {
            type = "app";
            program = "${self.nixosConfigurations.${system}.vm.config.system.build.vm}/bin/run-nixos-vm";
          };
          container = {
            type = "app";
            program = "${pkgs.writeShellScript "run-container" ''
              echo "Run on a NixOS host:"
              echo "1. Ensure containers are enabled in /etc/nixos/configuration.nix:"
              echo "   { boot.enableContainers = true; }"
              echo "2. Apply: sudo nixos-rebuild switch"
              echo "3. Create/start container: sudo nixos-container create dummy --flake .#container"
              echo "4. Start container: sudo nixos-container start dummy"
              echo "5. Access: sudo nixos-container run dummy -- /bin/bash"
            ''}";
          };
        };
      }
    );
}
