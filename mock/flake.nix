{
  description = "Mock services for manual testing of pcr stack";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.client = pkgs.writeShellScriptBin "mock-client" ''
        exec ${pkgs.python3}/bin/python3 ${./services/client.py} "client"
      '';

      stack = {
        watch.enable = true;
        logs = {
          dir = "./logs";
          max_lines = 10;
        };

        services = {
          # ── oneShot: runs and exits ──
          migrate = {
            cmd = [ "echo" "mock migration complete" ];
            oneShot = true;
          };

          # ── long-running: TCP listener on port 8080 ──
          server = {
            cmd = [ "nc" "-lk" "8080" ];
            ports = [ 8080 ];
            dependsOn = [ "migrate" ];
          };

          # ── long-running: simple log tail simulation ──
          worker = {
            cmd = [
              "bash"
              "-c"
              ''
                while true; do
                  echo "processing at $(date)";
                  sleep 3;
                done
              ''
            ];
            dependsOn = [ "migrate" ];
          };

          # ── long-running: Python client, packaged via Nix ──
          client = {
            cmd = [ "nix" "run" ".#client" "--" "client" ];
            src = "./services";
            dependsOn = [ "server" "migrate" ];
          };
        };
      };
    };
}
