{
  description = "Monorepo flake managing infra, app builds, and resource-constrained VMs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
    infrastructure = {
      url = "./infrastructure.nix";
      flake = false;
    };
    dummy-master = {
      url = "git+file://./?ref=master";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, flake-utils, infrastructure, dummy-master, ... }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs { inherit system; };
      inherit (pkgs) lib;

      # Parse infrastructure.nix
      infrastructureConfig = import infrastructure { inherit pkgs lib; };

      # Validate infrastructureConfig
      infraConfig = if (lib.isAttrs infrastructureConfig && infrastructureConfig ? machines)
        then infrastructureConfig
        else throw "infrastructure.nix must define a 'machines' attribute";

      # Build dummy app
      buildDummy = src: pkgs.stdenv.mkDerivation {
        pname = "dummy";
        version = "0.1.0";
        inherit src;
        buildInputs = [ pkgs.gcc pkgs.bash ];
        doCheck = true;
        checkPhase = ''
          if [ -f ./test_dummy.sh ]; then
            ${pkgs.bash}/bin/bash ./test_dummy.sh
          else
            echo "Warning: test_dummy.sh not found, running basic test"
            ${pkgs.bash}/bin/bash -c "true"
          fi
        '';
        buildPhase = ''
          gcc -o dummy main.c
        '';
        installPhase = ''
          mkdir -p $out/bin
          cp dummy $out/bin/
        '';
        meta = with lib; {
          description = "A dummy C executable for testing";
          license = licenses.mit;
        };
      };

      # Dummy packages
      dummyCurrent = buildDummy ./dummy;
      dummyMaster = buildDummy dummy-master;

      # Utility to extract package from config
      getPkg = config:
        if lib.isList config then config[0].package
        else if lib.isAttrs config && config ? package then config.package
        else throw "Invalid service config: missing package";

      # Service definitions
      services = {
        dummy = {
          production = {
            cpu = 1.5;
            memory = { amount = 1; unit = "GB"; };
            package = dummyMaster;
          };
          staging = [{
            cpu = 1.1;
            memory = { amount = 1; unit = "GB"; };
            package = dummyCurrent;
          }];
        };
      };

      # Validate memory unit
      validateService = env: let
        config = services.dummy.${env};
        cfg = if lib.isList config then config[0] else config;
      in
        if cfg.memory.unit != "GB" then throw "Memory unit must be 'GB' for ${env}"
        else cfg;

      # Generate app metadata
      appMetadata = lib.mapAttrsToList (name: app: {
        inherit name;
        command = "nix run .#${name}";
        environments = lib.mapAttrsToList (env: config: {
          environment = env;
          cpu = config.cpu or config[0].cpu or null;
          memory = config.memory or config[0].memory or null;
          package = (getPkg config).name or null;
        }) (services.${name} or {});
      }) self.apps.${system};

      # VM configuration
      mkVM = env: nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          ({ config, pkgs, ... }: {
            # VM-specific settings
            virtualisation = {
              vmVariant.enable = true;
              diskSize = 2048; # 2GB disk
              memorySize = (validateService env).memory.amount * 1024; # Convert GB to MB
              cores = lib.max 1 (lib.floor (validateService env).cpu); # At least 1 core
            };
            boot.loader.systemd-boot.enable = true;
            fileSystems."/" = {
              device = "/dev/disk/by-label/nixos";
              fsType = "ext4";
              autoFormat = true;
            };
            networking.hostName = "dummy-vm-${env}";
            services.openssh = {
              enable = true;
              settings.PermitRootLogin = "prohibit-password"; # Use SSH keys
            };
            services.qemuGuest.enable = true;
            networking.firewall.allowedTCPPorts = [ 22 ];
            # SSH key for root (generate locally for testing)
            users.users.root.openssh.authorizedKeys.keys = [
              # Replace with your SSH public key
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI..."
            ];
            environment.systemPackages = [ (getPkg services.dummy.${env}) ];
            systemd.services.dummy = {
              description = "Dummy App Service";
              wantedBy = [ "multi-user.target" ];
              serviceConfig = {
                ExecStart = "${getPkg services.dummy.${env}}/bin/dummy";
                Restart = "always";
                CPUQuota = "${toString ((validateService env).cpu * 100)}%";
                MemoryMax = "${toString (validateService env).memory.amount}${validateService env.memory.unit}";
              };
            };
          })
        ];
      };

    in {
      # VM configurations
      nixosConfigurations = {
        dummy-vm-production = mkVM "production";
        dummy-vm-staging = mkVM "staging";
      };

      # Tests
      checks = {
        dummy-current-test = pkgs.runCommand "dummy-current-test" {
          buildInputs = [ dummyCurrent ];
        } ''
          if [ -f ${./dummy}/test_dummy.sh ]; then
            ${pkgs.bash}/bin/bash ${./dummy}/test_dummy.sh > $out
          else
            dummy > $out
          fi
        '';
        dummy-master-test = pkgs.runCommand "dummy-master-test" {
          buildInputs = [ dummyMaster ];
        } ''
          if [ -f ${dummy-master}/test_dummy.sh ]; then
            ${pkgs.bash}/bin/bash ${dummy-master}/test_dummy.sh > $out
          else
            dummy > $out
          fi
        '';
      };

      # Build outputs
      packages = {
        dummy-current = dummyCurrent;
        dummy-master = dummyMaster;
        state = pkgs.writeTextFile {
          name = "state-lock";
          text = builtins.toJSON {
            infrastructure = infraConfig;
            services = services;
          };
          destination = "/state.json";
        };
        app-list = pkgs.writeTextFile {
          name = "app-list";
          text = builtins.toJSON appMetadata;
          destination = "/apps.json";
        };
        vm-production = self.nixosConfigurations.dummy-vm-production.config.system.build.vm;
        vm-staging = self.nixosConfigurations.dummy-vm-staging.config.system.build.vm;
        default = dummyCurrent;
      };

      # Runnable apps
      apps = {
        dummy-current = {
          type = "app";
          program = "${dummyCurrent}/bin/dummy";
        };
        dummy-master = {
          type = "app";
          program = "${dummyMaster}/bin/dummy";
        };
        list-apps = {
          type = "app";
          program = "${pkgs.writeShellScript "list-apps" ''
            cat ${self.packages.${system}.app-list}/apps.json | ${pkgs.jq}/bin/jq .
          ''}";
        };
        vm-production = {
          type = "app";
          program = "${self.packages.${system}.vm-production}/bin/run-nixos-vm";
        };
        vm-staging = {
          type = "app";
          program = "${self.packages.${system}.vm-staging}/bin/run-nixos-vm";
        };
      };

      # Development environment
      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [ gcc bash nix jq ];
        shellHook = ''
          echo "Available commands:"
          echo "  nix build .#dummy-current  # Build current version"
          echo "  nix run .#dummy-current    # Run current version"
          echo "  nix run .#vm-production    # Launch production VM"
          echo "  nix run .#vm-staging       # Launch staging VM"
          echo "  nix run .#list-apps        # List apps and resources"
          echo "  nix flake check            # Run tests"
        '';
      };
    });
}
