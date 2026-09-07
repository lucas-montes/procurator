{
  description = "Mock services for manual testing of pcr stack";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  };

  outputs = { self, nixpkgs }: {
    stack = {
      watch.enable = true;
      logs = {
        dir = "./logs";
        max_lines = 10;
      };

      services = {
        # ── oneShot: runs and exits ──
        migrate = {
          cmd = ["echo" "mock migration complete"];
          oneShot = true;
        };

        # ── long-running: TCP listener on port 8080 ──
        server = {
          cmd = ["nix" "run" "nixpkgs#python3" "--" "."];
          src = "./services/server";
          ports = [8080];
          dependsOn = ["migrate"];
          healthcheck = {
            test = ["CMD-SHELL" "ss -tln | grep -q 8080 || exit 1"];
            interval_secs = 10;
            timeout_secs = 5;
            retries = 2;
          };
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
          dependsOn = ["migrate"];
        };

        # ── long-running: Python client, runs via nixpkgs interpreter ──
        client = {
          cmd = ["nix" "run" "nixpkgs#python3" "--" "." "client"];
          src = "./services/client";
          dependsOn = ["server" "migrate"];
        };
      };
    };
  };
}
