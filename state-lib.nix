{ pkgs }:

let
  # Function to generate state.lock from a given config
  generateStateLock = { config }:
    let
      configJson = builtins.toJSON config;
      configHash = builtins.hashString "sha256" configJson;
    in
      pkgs.writeTextFile {
        name = "state-lock";
        text = builtins.toJSON {
          hash = configHash;
          config = config;
        };
        destination = "/state.lock";
      };

  # Function to validate state.lock file
  validateStateLock = { stateLock }:
    pkgs.runCommand "validate-state-lock" { } ''
      ${pkgs.jq}/bin/jq . ${stateLock}/state.lock > /dev/null || exit 1
      mkdir -p $out
    '';

  # Example config (can be overridden by caller)
  defaultConfig = {
    continuous-delivery = {
      tests = true;
      build = true;
      dst = true;
      staging = true;
    };
    machines = [
      {
        name = "victoria";
        cpu = 1;
        memory = {
          amount = 1;
          unit = "GB";
        };
        roles = [ "tests" "build" "monitoring" ];
      }
      {
        name = "tauri";
        cpu = 3;
        memory = {
          amount = 3;
          unit = "GB";
        };
        roles = [ "production" "DST" "staging" ];
      }
    ];
    rollback = {
      enabled = true;
      threshold = {
        cpu = 0.5;
        memory = {
          amount = 0.5;
          unit = "GB";
        };
        latency = {
          p99 = "100ms";
          p90 = "50ms";
          p50 = "20ms";
        };
      };
      notification = {
        enabled = true;
        email = {
          subject = "Rollback Notification";
          body = "A rollback has been initiated due to a failure in the deployment process.";
          recipients = [];
        };
        slack = {
          channel = "#rollbacks";
          message = "A rollback has been initiated due to a failure in the deployment process.";
          webhookUrl = "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX";
        };
      };
    };
    services = {
      dummy = {
        production = {
          cpu = 1.5;
          memory = {
            amount = 1;
            unit = "GB";
          };
          packages = "default";
        };
        staging = [
          {
            cpu = 1.1;
            memory = {
              amount = 1;
              unit = "GB";
            };
            packages = "default";
          }
        ];
      };
    };
  };

in {
  inherit generateStateLock validateStateLock defaultConfig;
}
