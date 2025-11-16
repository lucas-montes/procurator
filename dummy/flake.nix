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

        apps = {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/dummy";
            meta = with pkgs.lib; {
              description = "A dummy C executable for testing";
              license = licenses.mit;
            };
          };

        };
      }
    );
}
