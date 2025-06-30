
{
  services = {
      # Define new services that point to a custom package
      dummy = {
        production = {
          cpu = 1.5;
          memory = {
            amount = 1;
            unit = "GB";
          };
          packages = self.packages.${system}.default;
        };
        staging = [
          {
            cpu = 1.1;
            memory = {
              amount = 1;
              unit = "GB";
            };
            packages = self.packages.${system}.default;
          }
        ];
      };
    };
  # Define the services to run in the cluster
# Define the infrastructure for the cluster, machines and their roles
  infrastructure = {
  continous-delivery = {
    tests = true;
    build = true;
    dst = true;
    staging = true;
  };
  # Define the machines, their names, specs and roles so tasks are assinged correctly to them
  machines = [
    {
      name = "victoria";
      cpu = 1;
      memory = {
        amount = 1;
        unit = "GB";
      };
      roles = ["tests" "build" "monitoring"];
    }
    {
      name = "tauri";
      cpu = 3;
      memory = {
        amount = 3;
        unit = "GB";
      };
      roles = ["production" "DST" "staging"];
    }
  ];

  # The rollback strategy is defined by the threshold values, if the values are not met when running the DST, the rollback is initiated
  # The notification is sent to the different channels
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
};
}
