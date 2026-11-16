# Fast test for the CI remote-builder NixOS module — pure eval, no build.
#
# Validates:
#   1. The builder user exists, is a Nix trusted user, and its keys may only
#      run `nix-daemon --stdio`
#   2. Caches become substituters only when given
#   3. No authorized key is an assertion failure
#
# Run:
#   nix eval -f ./nix/tests/ci-builder-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>), system ? "x86_64-linux" }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) mkPkgsStub stubSystemd checkAssertions assert' assertEq assertThrows;

  pkgsStub = mkPkgsStub { packages = [ ]; inherit system; };

  evalBuilder = builderCfg:
    lib.evalModules {
      modules = [
        ../modules/ci-builder.nix
        stubSystemd
        ({ ... }: { config.services.procurator.ciBuilder = builderCfg; })
      ];
      specialArgs = { pkgs = pkgsStub; };
    };

  key = "ssh-ed25519 AAAAC3Nza root@ci";

  plain = evalBuilder { enable = true; authorizedKeys = [ key ]; };
  plainCheck =
    assert' "passes assertions" (checkAssertions plain)
    && assertEq "keys are restricted to the daemon"
      plain.config.users.users.nix-builder.openssh.authorizedKeys.keys
      [ ''restrict,command="nix-daemon --stdio" ${key}'' ]
    && assertEq "trusted user" plain.config.nix.settings.trusted-users [ "nix-builder" ]
    && assert' "no substituters" (!(plain.config.nix.settings ? extra-substituters))
    && assert' "sshd on" plain.config.services.openssh.enable
    && assert' "group exists" (plain.config ? users.groups.nix-builder);

  cached = evalBuilder {
    enable = true;
    user = "builder";
    authorizedKeys = [ key ];
    substituters = [ "http://cache.lab:8081/public" ];
    trustedPublicKeys = [ "cache.local:abc=" ];
  };
  cachedCheck =
    assertEq "substituters" cached.config.nix.settings.extra-substituters [ "http://cache.lab:8081/public" ]
    && assertEq "keys" cached.config.nix.settings.extra-trusted-public-keys [ "cache.local:abc=" ]
    && assertEq "custom user trusted" cached.config.nix.settings.trusted-users [ "builder" ];

  keyless = assertThrows "no key fails"
    (checkAssertions (evalBuilder { enable = true; }));
in
plainCheck && cachedCheck && keyless
