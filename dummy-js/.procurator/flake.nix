

{
  description = "Auto-generated flake for api, web";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }@inputs:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages = {
          
          procurator-web = pkgs.stdenv.mkDerivation {
            pname = "procurator-web";
            version = "latest";
            src = ./.;

            buildInputs = with pkgs; [
              javascript
              
            ];

            meta = with pkgs.lib; {
              description = "";
              
              maintainers = [
                
              ];
            };
          };
          
          procurator-api = pkgs.stdenv.mkDerivation {
            pname = "procurator-api";
            version = "latest";
            src = ./.;

            buildInputs = with pkgs; [
              javascript
              
            ];

            meta = with pkgs.lib; {
              description = "";
              
              maintainers = [
                
              ];
            };
          };
          
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            
            javascript
            
            
          ];

          
        };

        checks = {
          
        };
      }
    ) // {
      procurator = {
        services = [
          
        ];

        project = {
          name = "api";
          languages = [
            
            "javascript"
            
          ];
          packageManagers = [
            
            "npm"
            
          ];
        };
      };
    };
}