{
  description = "Procurator — web and api dev shells and packages";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs { inherit system; overlays = []; };
      node = pkgs.nodejs-20_x;
      nodePackages = pkgs.nodePackages;
      makeShell = drv: pkgs.mkShell {
        buildInputs = [ node pkgs.git pkgs.nodePackages.node2nix ];
      };
    in
    {
      packages = {
        web = pkgs.stdenv.mkDerivation {
          pname = "procurator-web";
          src = ./apps/web;
          buildInputs = [ node pkgs.yarn pkgs.nodePackages.node2nix ];
          buildPhase = ''
            npm ci --legacy-peer-deps
            npm run build
          '';
          installPhase = ''
            mkdir -p $out
            cp -r dist/* $out/
          '';
        };

        api = pkgs.stdenv.mkDerivation {
          pname = "procurator-api";
          src = ./apps/api;
          buildInputs = [ node pkgs.nodePackages.node2nix ];
          buildPhase = ''
            npm ci --legacy-peer-deps
            npm run build
          '';
          installPhase = ''
            mkdir -p $out
            cp -r dist $out/
          '';
        };
      };

      devShells = {
        web = makeShell {
          name = "procurator-web-shell";
        };
        api = makeShell {
          name = "procurator-api-shell";
        };
      };

      defaultPackage = packages.web;
    })
}
