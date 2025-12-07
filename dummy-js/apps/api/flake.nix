{
  description = "Procurator api flake (Fastify + TypeScript)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.11";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" ];
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = pkgsFor system .mkShell {
          buildInputs = [ pkgsFor system .nodejs-20_x pkgsFor system .git pkgsFor system .yarn pkgsFor system .bashInteractive ];
          shellHook = ''
            echo "Entering dev shell for procurator api (Fastify)"
            echo "Run: npm run dev"
          '';
        };
      }) systems);

      packages = builtins.listToAttrs (map (system: {
        name = system;
        value = pkgsFor system .stdenv.mkDerivation {
          pname = "procurator-api";
          src = ./.;
          nativeBuildInputs = [ pkgsFor system .nodejs-20_x ];
          buildPhase = ''
            npm ci --legacy-peer-deps --prefix "$src"
            npm run build --prefix "$src"
          '';
          installPhase = ''
            mkdir -p $out
            cp -r "$src/dist" $out/
          '';
        };
      }) systems);

      defaultPackage = packages.x86_64-linux;
    };
}
