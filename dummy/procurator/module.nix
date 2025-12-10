# Procurator Module System
# This provides a NixOS-style module for declaring infrastructure

{ lib, ... }:
let
  inherit (lib) mkOption types mkEnableOption;

  # Memory specification type
  memoryType = types.submodule {
    options = {
      amount = mkOption {
        type = types.float;
        description = "Amount of memory";
      };
      unit = mkOption {
        type = types.enum [ "MB" "GB" ];
        default = "GB";
        description = "Memory unit";
      };
    };
  };

  # Machine specification type
  machineType = types.submodule {
    options = {
      cpu = mkOption {
        type = types.float;
        description = "Number of CPU cores";
      };
      memory = mkOption {
        type = memoryType;
        description = "Memory specification";
      };
      roles = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Roles this machine can perform";
      };
    };
  };

  # Environment configuration type
  environmentType = types.submodule {
    options = {
      cpu = mkOption {
        type = types.float;
        description = "CPU allocation for this environment";
      };
      memory = mkOption {
        type = memoryType;
        description = "Memory allocation";
      };
      replicas = mkOption {
        type = types.int;
        default = 1;
        description = "Number of replicas";
      };
      healthCheck = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Health check command or HTTP endpoint";
        example = "/health";
      };
    };
  };

  # Service type - supports flake, source URL, or inline package
  serviceType = types.submodule ({ name, config, ... }: {
    options = {
      # Option A: Direct flake reference (from inputs)
      flake = mkOption {
        type = types.nullOr types.attrs;
        default = null;
        description = "Flake reference from inputs";
        example = "inputs.my-service";
      };

      # Option B: Git URL (resolved at deploy time)
      source = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Git URL to the service flake";
        example = "github:myorg/my-service";
      };

      # Option C: Inline package (for monorepo)
      package = mkOption {
        type = types.nullOr types.package;
        default = null;
        description = "Direct package reference";
      };

      # Pinning for source option
      revision = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Pin to specific git revision (for source option)";
        example = "v1.2.3";
      };

      # Which output to use from the flake
      output = mkOption {
        type = types.str;
        default = "default";
        description = "Which flake output to deploy";
        example = "packages.x86_64-linux.server";
      };

      # Environment configurations
      environments = mkOption {
        type = types.attrsOf environmentType;
        default = { };
        description = "Environment-specific configurations";
        example = {
          production = { cpu = 2.0; memory = { amount = 2; }; };
          staging = { cpu = 1.0; memory = { amount = 1; }; };
        };
      };

      # Computed: resolve the actual package
      resolvedPackage = mkOption {
        type = types.nullOr types.package;
        internal = true;
        default =
          if config.package != null then config.package
          else if config.flake != null then
            lib.attrByPath (lib.splitString "." config.output) null config.flake
          else null; # source URLs resolved at deploy time
      };

      # Computed: the source info for the control plane
      sourceInfo = mkOption {
        type = types.attrs;
        internal = true;
        default = {
          type =
            if config.package != null then "package"
            else if config.flake != null then "flake"
            else "url";
          url = config.source;
          rev = config.revision;
          output = config.output;
        };
      };
    };
  });

  # Latency threshold type
  latencyThresholdType = types.submodule {
    options = {
      p99 = mkOption { type = types.nullOr types.str; default = null; };
      p90 = mkOption { type = types.nullOr types.str; default = null; };
      p50 = mkOption { type = types.nullOr types.str; default = null; };
    };
  };

  # Rollback threshold type
  rollbackThresholdType = types.submodule {
    options = {
      cpu = mkOption {
        type = types.nullOr types.float;
        default = null;
        description = "CPU threshold for rollback";
      };
      memory = mkOption {
        type = types.nullOr memoryType;
        default = null;
        description = "Memory threshold for rollback";
      };
      latency = mkOption {
        type = types.nullOr latencyThresholdType;
        default = null;
        description = "Latency thresholds for rollback";
      };
    };
  };

  # Email notification type
  emailConfigType = types.submodule {
    options = {
      subject = mkOption { type = types.str; };
      body = mkOption { type = types.str; };
      recipients = mkOption { type = types.listOf types.str; default = [ ]; };
    };
  };

  # Slack notification type
  slackConfigType = types.submodule {
    options = {
      channel = mkOption { type = types.str; };
      message = mkOption { type = types.str; };
      webhookUrl = mkOption { type = types.str; };
    };
  };

  # Notification configuration type
  notificationConfigType = types.submodule {
    options = {
      enabled = mkEnableOption "notifications";
      email = mkOption {
        type = types.nullOr emailConfigType;
        default = null;
      };
      slack = mkOption {
        type = types.nullOr slackConfigType;
        default = null;
      };
    };
  };

  # Rollback configuration type
  rollbackConfigType = types.submodule {
    options = {
      enabled = mkEnableOption "automatic rollback";
      threshold = mkOption {
        type = types.nullOr rollbackThresholdType;
        default = null;
      };
      notification = mkOption {
        type = types.nullOr notificationConfigType;
        default = null;
      };
    };
  };

  # CD pipeline configuration
  cdConfigType = types.submodule {
    options = {
      tests = mkEnableOption "run tests";
      build = mkEnableOption "build packages";
      dst = mkEnableOption "distributed system tests";
      staging = mkEnableOption "staging deployment";
    };
  };
in
{
  options.procurator = {
    enable = mkEnableOption "Procurator infrastructure management";

    machines = mkOption {
      type = types.attrsOf machineType;
      default = { };
      description = "Machine definitions for the cluster";
      example = {
        victoria = {
          cpu = 1;
          memory = { amount = 1; unit = "GB"; };
          roles = [ "tests" "build" ];
        };
      };
    };

    services = mkOption {
      type = types.attrsOf serviceType;
      default = { };
      description = "Service definitions";
    };

    cd = mkOption {
      type = cdConfigType;
      default = { };
      description = "Continuous delivery pipeline configuration";
    };

    rollback = mkOption {
      type = rollbackConfigType;
      default = { };
      description = "Rollback configuration";
    };
  };
}
