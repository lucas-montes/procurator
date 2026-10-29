{
  description = "Small Rust project that pushes to and pulls from a local Nix binary cache";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "local-cache-example";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
      };

      devShells.${system}.default = pkgs.mkShell {
        buildInputs = [ pkgs.cargo pkgs.rustc ];
      };
    };

  nixConfig = {
    # Equivalent to:
    # nix.settings.substituters = [ "http://127.0.0.1:8081" ];
    # nix.settings.trusted-public-keys = [ "cache.local-1:REPLACE_WITH_YOUR_PUBLIC_KEY_BASE64" ];
    # nix.settings.accept-flake-config = true;
    substituters = [ "http://127.0.0.1:8081" ];
    trusted-public-keys = [ "cache.local-1:REPLACE_WITH_YOUR_PUBLIC_KEY_BASE64" ];
    accept-flake-config = true;
  };
}
