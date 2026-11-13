{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.cache;
  tomlFormat = pkgs.formats.toml { };
  # Why: single TOML file is the server's only config source (no env vars).
  configFile = tomlFormat.generate "procurator-cache.toml" ({
    addr = cfg.addr;
    storage_dir = toString cfg.storageDir;
    gc = {
      interval_secs = cfg.gc.intervalSecs;
      orphan_grace_secs = cfg.gc.orphanGraceSecs;
    };
    caches = mapAttrs (_: c: {
      kind = c.kind;
      visibility = c.visibility;
      sign_key_file = toString c.signKeyFile;
      store_dir = c.storeDir;
      priority = c.priority;
      max_size = c.maxSize;
      retention_days = c.retentionDays;
    }) cfg.caches;
  } // optionalAttrs (cfg.adminTokenSha256 != null) {
    admin_token_sha256 = cfg.adminTokenSha256;
  });

  cacheOptions = {
    options = {
      kind = mkOption {
        type = types.enum [ "nix" ];
        default = "nix";
        description = "Protocol this cache speaks. Only Nix binary caches for now.";
      };

      visibility = mkOption {
        type = types.enum [ "public" "private" ];
        example = "private";
        description = "public = anyone reads, a token with a write grant pushes; private = reading and pushing both need a token.";
      };

      signKeyFile = mkOption {
        type = types.path;
        example = "/run/secrets/cache.secret";
        description = "Ed25519 signing key (nix-store --generate-binary-cache-key format), readable by the service user. Every stored narinfo is signed with it.";
      };

      storeDir = mkOption {
        type = types.str;
        default = "/nix/store";
        description = "Store prefix of the paths this cache serves.";
      };

      priority = mkOption {
        type = types.ints.unsigned;
        default = 30;
        description = "Advertised substituter priority (lower = preferred).";
      };

      maxSize = mkOption {
        type = types.str;
        default = "0";
        example = "100G";
        description = "Quota for this cache (e.g. 50G, 0 = unlimited). Uploads beyond it get 507; GC evicts least-recently-read paths above 90%.";
      };

      retentionDays = mkOption {
        type = types.ints.unsigned;
        default = 0;
        example = 90;
        description = "Days a path is kept after it was last read (0 = until space is needed).";
      };
    };
  };
in {
  options.services.procurator.cache = {
    enable = mkEnableOption "Procurator artifact cache (named public/private Nix caches, token auth)";

    package = mkOption {
      type = types.package;
      default = pkgs.cache;
      defaultText = literalExpression "pkgs.cache";
      description = "The cache package to use.";
    };

    addr = mkOption {
      type = types.str;
      default = "0.0.0.0:8081";
      example = "127.0.0.1:8081";
      description = "Address and port to bind. The service speaks plain HTTP: terminate TLS in a reverse proxy in front of it.";
    };

    storageDir = mkOption {
      type = types.path;
      default = "/var/lib/procurator-cache";
      description = "Directory for blobs and the SQLite index.";
    };

    adminTokenSha256 = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "1734d503f6aa6a047c36d113cbad769f719c93784b469b771c4c3e7c63adbefd";
      description = "Hex SHA-256 of the bootstrap admin token, which mints all other tokens through /_api/v1/tokens. The hash is safe to commit; the token is not. Null = tokens can be used but not created.";
    };

    gc = {
      intervalSecs = mkOption {
        type = types.ints.positive;
        default = 3600;
        description = "Seconds between garbage-collection passes.";
      };

      orphanGraceSecs = mkOption {
        type = types.ints.unsigned;
        default = 86400;
        description = "Seconds an uploaded NAR may wait for its narinfo before GC removes it.";
      };
    };

    caches = mkOption {
      type = types.attrsOf (types.submodule cacheOptions);
      default = {};
      example = literalExpression ''
        {
          public = { visibility = "public"; signKeyFile = "/run/secrets/public.key"; };
          internal = { visibility = "private"; signKeyFile = "/run/secrets/internal.key"; maxSize = "200G"; retentionDays = 90; };
        }
      '';
      description = "Named caches, each served under /<name>/.";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open the firewall for the configured port.";
    };

    user = mkOption {
      type = types.str;
      default = "procurator-cache";
      description = "User account under which the cache service runs.";
    };

    group = mkOption {
      type = types.str;
      default = "procurator-cache";
      description = "Group under which the cache service runs.";
    };

    extraArgs = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Additional command-line arguments to pass to the cache service.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.caches != {};
        message = "services.procurator.cache.caches must define at least one cache.";
      }
      {
        assertion = cfg.adminTokenSha256 == null
          || builtins.match "[0-9a-f]{64}" cfg.adminTokenSha256 != null;
        message = "services.procurator.cache.adminTokenSha256 must be 64 lowercase hex characters.";
      }
    ];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Procurator cache service user";
      home = cfg.storageDir;
    };

    users.groups.${cfg.group} = {};

    systemd.tmpfiles.rules = [
      "d ${cfg.storageDir} 0750 ${cfg.user} ${cfg.group} -"
    ];

    # Why: parse the port from addr without extra options to keep.
    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall (
      let m = builtins.match ".*:([0-9]+)" cfg.addr;
      in if m == null then [] else [ (builtins.fromJSON (builtins.head m)) ]
    );

    systemd.services.procurator-cache = {
      description = "Procurator Artifact Cache";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/cache --config ${configFile} ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Why: the service only ever writes its own storage directory.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.storageDir ];
        StateDirectory = "procurator-cache";
      };
    };
  };
}
