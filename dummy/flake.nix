{
  description = "This flake will simulate a simple monorepo where infra and code are in the same repo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
    flake-utils.url = "github:numtide/flake-utils";
    # Example: external service as flake input (pinned via lock file)
    # auth-service.url = "github:myorg/auth-service";
  };

  #   nixConfig = {
  #   substituters = ["http://0.0.0.0:8081"];
  #   post-build-hook = "${./upload-hook.sh}";
  # };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    ...
  }:
    let
      lib = nixpkgs.lib;

      # Evaluate procurator config for a given system
      evalProcurator = system: pkgs: lib.evalModules {
        modules = [
          ./procurator/module.nix
          ./procurator/config.nix
          {
            _module.args = {
              inherit pkgs;
              packages = self.packages.${system};
              # inputs = { inherit auth-service; };
            };
          }
        ];
      };
    in
    {
      # Export the module for reuse
      nixosModules.procurator = import ./procurator/module.nix;
    }
    //
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
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

        # Import CI-specific configuration
        ciConfig = import ./ci.nix {inherit pkgs;};

        # Evaluate procurator configuration
        procuratorEval = evalProcurator system pkgs;
      in {
        packages = {
          default = buildDummy;
          dummy = buildDummy;

          # This package allows us to run build and have the state generated
          state = pkgs.writeTextFile {
            name = "state-lock";
            text = builtins.toJSON {
              machines = procuratorEval.config.procurator.machines;
              services = lib.mapAttrs (name: svc: {
                sourceInfo = svc.sourceInfo;
                environments = svc.environments;
              }) procuratorEval.config.procurator.services;
              cd = procuratorEval.config.procurator.cd;
              rollback = procuratorEval.config.procurator.rollback;
            };
            destination = "/state.json";
          };
        };

        checks =
          {
            # Existing basic test
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
          }
          // ciConfig.checks; # Merge CI-specific checks

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

        # Export infrastructure configuration (JSON-serializable)
        infrastructure = {
          machines = procuratorEval.config.procurator.machines;
          services = lib.mapAttrs (name: svc: {
            sourceInfo = svc.sourceInfo;
            environments = svc.environments;
          }) procuratorEval.config.procurator.services;
          cd = procuratorEval.config.procurator.cd;
          rollback = procuratorEval.config.procurator.rollback;
        };
      }
    );
}
