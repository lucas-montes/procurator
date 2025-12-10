# Procurator Infrastructure Configuration
# This file uses the module system to declare your infrastructure

{ pkgs, packages, lib, ... }:

{
  procurator = {
    enable = true;

    # Define the machines in your cluster
    machines = {
      victoria = {
        cpu = 1;
        memory = { amount = 1; unit = "GB"; };
        roles = [ "tests" "build" "monitoring" ];
      };
      tauri = {
        cpu = 3;
        memory = { amount = 3; unit = "GB"; };
        roles = [ "production" "DST" "staging" ];
      };
    };

    # Define services
    services = {
      # Option 1: Inline package (monorepo style)
      dummy = {
        package = packages.default;
        environments = {
          production = {
            cpu = 1.5;
            memory = { amount = 1; unit = "GB"; };
            replicas = 2;
            healthCheck = "/health";
          };
          staging = {
            cpu = 1.0;
            memory = { amount = 512; unit = "MB"; };
          };
        };
      };

      # Option 2: External flake (uncomment when you have auth-service input)
      # auth = {
      #   flake = inputs.auth-service;
      #   output = "default";
      #   environments = {
      #     production = {
      #       cpu = 1.0;
      #       memory = { amount = 1; unit = "GB"; };
      #     };
      #   };
      # };

      # Option 3: URL reference (resolved at deploy time)
      # worker = {
      #   source = "github:myorg/worker-service";
      #   revision = "v1.2.3";
      #   environments = {
      #     production = {
      #       cpu = 2.0;
      #       memory = { amount = 2; unit = "GB"; };
      #       replicas = 5;
      #     };
      #   };
      # };
    };

    # CD pipeline configuration
    cd = {
      tests = true;
      build = true;
      dst = true;
      staging = true;
    };

    # Rollback configuration
    rollback = {
      enabled = true;
      threshold = {
        cpu = 0.8;
        memory = { amount = 0.9; unit = "GB"; };
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
          body = "A rollback has been initiated due to threshold breach.";
          recipients = [ ];
        };
        slack = {
          channel = "#deployments";
          message = "Rollback initiated";
          webhookUrl = "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX";
        };
      };
    };
  };
}
