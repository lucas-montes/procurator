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
    # Why `extra-`: the bare `substituters`/`trusted-public-keys` keys REPLACE
    # the system defaults. Dropping cache.nixos.org makes a plain `nix build`
    # here try to bootstrap stdenv from source (~900 derivations). The `extra-`
    # forms append, so the local cache is consulted *in addition to* upstream.
    #
    # Equivalent NixOS form:
    #   nix.settings.extra-substituters = [ "http://127.0.0.1:8081/public" ];
    #   nix.settings.extra-trusted-public-keys = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
    extra-substituters = [ "http://127.0.0.1:8081/public" ];
    # This is the throwaway dev key committed at ../cache/cache-public.pem, so
    # the example works out of the box. Generate your own before trusting this
    # cache for anything real — the matching secret key is in the repo.
    extra-trusted-public-keys = [ "cache.local:hfNlzq2QP6JLd9ghjD/6+IZ3A21etwghcrKP5wvJV2k=" ];
    accept-flake-config = true;
  };
}
