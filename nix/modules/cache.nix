{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.procurator.cache;
  tomlFormat = pkgs.formats.toml { };
  # Why: single TOML file is the server's only config source (no env vars).
  configFile = tomlFormat.generate "procurator-cache.toml" ({
    addr = cfg.addr;
    store_dir = cfg.storeDir;
    storage_dir = toString cfg.storageDir;
    max_size = cfg.maxSize;
    priority = cfg.priority;
    sign_key_files = map toString cfg.secretKeyFiles;
    tls = { provider = cfg.tls.provider; }
      // optionalAttrs (cfg.tls.certFile != null) { cert_file = toString cfg.tls.certFile; }
      // optionalAttrs (cfg.tls.keyFile != null) { key_file = toString cfg.tls.keyFile; }
      // optionalAttrs (cfg.tls.clientCaFile != null) { client_ca_file = toString cfg.tls.clientCaFile; }
      // { writer_allowlist = cfg.tls.writerAllowlist; };
    access = {
      reads_require_auth = cfg.access.readsRequireAuth;
      writes_require_auth = cfg.access.writesRequireAuth;
      loopback_bypass = cfg.access.loopbackBypass;
    };
  });
in {
  options.services.procurator.cache = {
    enable = mkEnableOption "Procurator Nix binary cache (read/write, mTLS)";

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
      description = "Address and port for the cache service to bind to.";
    };

    storeDir = mkOption {
      type = types.str;
      default = "/nix/store";
      description = "Local Nix store proxied on cache miss.";
    };

    storageDir = mkOption {
      type = types.path;
      default = "/var/lib/procurator-cache";
      description = "Directory for uploaded .narinfo and nar/ objects (LocalDisk backend).";
    };

    storageBackend = mkOption {
      type = types.enum [ "disk" ];
      default = "disk";
      description = "Storage backend. Only local disk for now; S3/GCS come later via the StorageBackend trait.";
    };

    maxSize = mkOption {
      type = types.str;
      default = "50G";
      example = "100G";
      description = "Soft quota for uploaded objects (e.g. 50G, 0 = unlimited).";
    };

    priority = mkOption {
      type = types.ints.unsigned;
      default = 30;
      description = "Advertised substituter priority (lower = preferred).";
    };

    secretKeyFiles = mkOption {
      type = types.listOf types.path;
      default = [];
      example = [ "/run/secrets/cache.secret" ];
      description = "Ed25519 cache signing keys (nix-store --generate-binary-cache-key format). Server signs uploads lacking Sig:.";
    };

    tls = {
      provider = mkOption {
        type = types.enum [ "file" ];
        default = "file";
        description = "TLS provider. Only file (PEM paths) for now; vault/step/spire come later via the TlsProvider trait.";
      };

      certFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/cache-cert.pem";
        description = "Server certificate chain (PEM). Null = plain HTTP (dev only).";
      };

      keyFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/cache-key.pem";
        description = "Server private key (PEM).";
      };

      clientCaFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/cache-client-ca.pem";
        description = "Client CA bundle (PEM). Set = require mTLS client certificates.";
      };

      writerAllowlist = mkOption {
        type = types.listOf types.str;
        default = [];
        example = [ "ci-writer" "pcr-push" ];
        description = "Allowed writer identities (CN or sha256:<hex fingerprint>). Empty = any verified client may write.";
      };
    };

    access = {
      readsRequireAuth = mkOption {
        type = types.bool;
        default = false;
        description = "Require a verified client even for GET/HEAD. Off keeps substituters usable.";
      };

      writesRequireAuth = mkOption {
        type = types.enum [ "auto" "always" "never" ];
        default = "auto";
        description = "Write policy: auto = open on loopback bind, required otherwise; always = require; never = open (refused at startup on public binds).";
      };

      loopbackBypass = mkOption {
        type = types.bool;
        default = true;
        description = "Trust 127.0.0.1/::1 without a client cert (co-located CI needs zero creds).";
      };
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
      example = [ "--verbose" ];
      description = "Additional command-line arguments to pass to the cache service.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = (cfg.tls.certFile == null) == (cfg.tls.keyFile == null);
        message = "services.procurator.cache.tls.certFile and keyFile must be set together.";
      }
      {
        assertion = cfg.tls.clientCaFile == null || cfg.tls.certFile != null;
        message = "services.procurator.cache.tls.clientCaFile (mTLS) requires certFile/keyFile.";
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
      "d ${cfg.storageDir}/nar 0750 ${cfg.user} ${cfg.group} -"
    ];

    # Why: parse the port from addr without extra options to keep.
    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall (
      let m = builtins.match ".*:([0-9]+)" cfg.addr;
      in if m == null then [] else [ (builtins.fromJSON (builtins.head m)) ]
    );

    systemd.services.procurator-cache = {
      description = "Procurator Nix Binary Cache (read/write, mTLS)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "nix-daemon.socket" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/cache --config ${configFile} ${concatStringsSep " " cfg.extraArgs}";
        Restart = "on-failure";
        RestartSec = "10s";

        # Why: read /nix/store + nix db, write only the cache dir.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.storageDir ];
        ReadOnlyPaths = [ "/nix/store" "/nix/var/nix/db" ];
        StateDirectory = "procurator-cache";
      };
    };
  };
}
