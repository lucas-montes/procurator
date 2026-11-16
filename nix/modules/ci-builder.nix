{ config, lib, ... }:

# A remote builder for the Procurator CI host (the CI side is
# services.procurator.ci.remoteBuilders). The CI host's nix-daemon logs in
# over SSH as `user` and runs `nix-daemon --stdio` (ssh-ng); nothing else.

with lib;

let
  cfg = config.services.procurator.ciBuilder;
  nixPackage = config.nix.package or null;
  daemon = if nixPackage == null then "nix-daemon" else "${nixPackage}/bin/nix-daemon";
  # Why: the key may only speak the Nix store protocol, never open a shell
  # or forward ports.
  restricted = key: ''restrict,command="${daemon} --stdio" ${key}'';
in {
  options.services.procurator.ciBuilder = {
    enable = mkEnableOption "a remote builder for the Procurator CI host";

    user = mkOption {
      type = types.str;
      default = "nix-builder";
      description = "User the CI host logs in as (its remoteBuilders.*.sshUser).";
    };

    authorizedKeys = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "ssh-ed25519 AAAA… root@ci" ];
      description = ''
        Public halves of the CI hosts' remoteBuilders.*.sshKey. Each may only
        run `nix-daemon --stdio`. The user is a Nix trusted user (remote
        builds need it), so these keys control this machine's Nix store:
        list only CI hosts.
      '';
    };

    substituters = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "http://cache.lan:8081/public" ];
      description = ''
        Caches this builder downloads dependencies from (the CI host sets
        builders-use-substitutes), e.g. the Procurator cache's public cache.
      '';
    };

    trustedPublicKeys = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Signing keys of the substituters above.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [{
      assertion = cfg.authorizedKeys != [ ];
      message = "services.procurator.ciBuilder.authorizedKeys must list the CI host's key.";
    }];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.user;
      description = "Procurator CI remote builder";
      # Why: sshd runs the forced command through the login shell.
      useDefaultShell = true;
      openssh.authorizedKeys.keys = map restricted cfg.authorizedKeys;
    };

    users.groups.${cfg.user} = { };

    services.openssh.enable = true;

    nix.settings = {
      # Why: the CI host sends derivations and inputs it built itself,
      # unsigned; only trusted users may add those to the store.
      trusted-users = [ cfg.user ];
    } // optionalAttrs (cfg.substituters != [ ]) {
      extra-substituters = cfg.substituters;
      extra-trusted-public-keys = cfg.trustedPublicKeys;
    };
  };
}
