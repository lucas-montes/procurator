{
  description = "NixOS VM with dummy app and resource constraints";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs { inherit system; };

      # Dummy app build
      dummy = pkgs.stdenv.mkDerivation {
        pname = "dummy";
        version = "0.1.0";
        src = ./dummy;

        buildInputs = [ pkgs.gcc ];

        buildPhase = ''
          gcc -o dummy main.c
        '';

        checkPhase = ''
          ./test_dummy.sh
        '';

        installPhase = ''
          mkdir -p $out/bin
          cp dummy $out/bin/
        '';
      };

      # Build a NixOS VM
      vm = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          ({ config, pkgs, ... }: {
            virtualisation.vmVariant = {
              enable = true;
              memorySize = 1024; # 1GB RAM
              cores = 1;
            };

            networking.hostName = "dummy-vm";

            services.qemuGuest.enable = true;
            services.openssh.enable = true;

            environment.systemPackages = [ dummy ];

            users.users.root.password = "root"; # Dev only

            systemd.services.dummy = {
              wantedBy = [ "multi-user.target" ];
              description = "Dummy service";
              serviceConfig = {
                ExecStart = "${dummy}/bin/dummy";
                Restart = "always";
                CPUQuota = "100%";
                MemoryMax = "512M";
              };
            };
          })
        ];
      };
    in {
      packages.vm = vm.config.system.build.vm;
      apps.vm = {
        type = "app";
        program = "${vm.config.system.build.vm}/bin/run-nixos-vm";
      };
      devShells.default = pkgs.mkShell {
        buildInputs = [ pkgs.gcc ];
      };
    });
}
