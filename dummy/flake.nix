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

        # Build Rust library for FFI
        rustLib = pkgs.rustPlatform.buildRustPackage {
          pname = "dummy-rust";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          # Build only the library, not the binary
          buildPhase = ''
            cargo build --release --lib
          '';
          installPhase = ''
            mkdir -p $out/lib
            cp target/release/libdummy_rust.a $out/lib/
            cp target/release/libdummy_rust.so $out/lib/ || true
          '';
        };

        # Build C program that uses Rust FFI (static linking)
        ffiExample = pkgs.stdenv.mkDerivation {
          pname = "ffi-example";
          version = "0.1.0";
          src = ./.;
          buildInputs = [pkgs.gcc];
          buildPhase = ''
            gcc -o ffi_example ffi_example.c \
              -L${rustLib}/lib \
              -ldummy_rust \
              -lpthread -ldl -lm
          '';
          installPhase = ''
            mkdir -p $out/bin
            cp ffi_example $out/bin/
          '';
        };

        # Build C program that uses Rust FFI (dynamic linking)
        ffiExampleDynamic = pkgs.stdenv.mkDerivation {
          pname = "ffi-example-dynamic";
          version = "0.1.0";
          src = ./.;
          buildInputs = [pkgs.gcc];
          buildPhase = ''
            if [ -f ${rustLib}/lib/libdummy_rust.so ]; then
              gcc -o ffi_example_dynamic ffi_example.c \
                -L${rustLib}/lib \
                -ldummy_rust \
                -Wl,-rpath,${rustLib}/lib
            else
              echo "Dynamic library not available, skipping"
              touch ffi_example_dynamic
            fi
          '';
          installPhase = ''
            mkdir -p $out/bin
            if [ -s ffi_example_dynamic ]; then
              cp ffi_example_dynamic $out/bin/
            fi
          '';
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
          default = ffiExample;
          dummy = buildDummy;
          rust-lib = rustLib;
          ffi-example = ffiExample;
          ffi-example-dynamic = ffiExampleDynamic;

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
            program = "${self.packages.${system}.ffi-example}/bin/ffi_example";
            meta = with pkgs.lib; {
              description = "FFI example calling Rust from C";
              license = licenses.mit;
            };
          };
          dummy = {
            type = "app";
            program = "${self.packages.${system}.dummy}/bin/dummy";
            meta = with pkgs.lib; {
              description = "A dummy C executable for testing";
              license = licenses.mit;
            };
          };
          ffi = {
            type = "app";
            program = "${self.packages.${system}.ffi-example}/bin/ffi_example";
            meta = with pkgs.lib; {
              description = "FFI example calling Rust from C";
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
