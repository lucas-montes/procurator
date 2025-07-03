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
              packages = "${./default.nix}";
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

        dummy = pkgs.stdenv.mkDerivation {
          pname = "dummy";
          version = "0.1.0";
          src = ./.;

          buildInputs = [pkgs.gcc];

          doCheck = true;
          checkPhase = ''
            # Run the tests
            ./test_dummy.sh
          '';

          buildPhase = ''
            gcc -o dummy main.c
          '';

          installPhase = ''
            mkdir -p $out/bin
            cp dummy $out/bin/
          '';

          meta = with pkgs.lib; {
            description = "A dummy C executable for testing";
            license = licenses.mit;
          };
        };
      in {
        checks = {
          inherit (self.packages.${system}) production staging;
        };
        packages = {
          # This package allow us to run build and have the state generated. Probably shouldn't be here?
          state = pkgs.writeTextFile {
            name = "state-lock";
            text = builtins.toJSON {
              inherit infrastructure services;
            };
            destination = "/state.json";
          };

          production = dummy;
          staging = dummy;
        };

        devShells.default = with pkgs;
          mkShell {
            buildInputs = [
            ];
          };
      }
    );
}
