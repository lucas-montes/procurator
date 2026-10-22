# NixOS system-eval test — the five services assembled into one real NixOS
# system. Pure evaluation of the NixOS module system (no build, no VM):
# proves the modules compose, wire their users/groups/units/config files,
# and pass all cross-module assertions under the real option semantics
# (not the eval-test stubs used by the *-fast.nix tests).
#
# Requires `pkgs` with the flake overlay applied (pkgs.cache etc.).
#
# Returns true on success; throws on the first broken module.
{ lib, pkgs }:
let
  nixosEval = lib.nixosSystem {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [
      {
        nixpkgs.pkgs = pkgs;
        system.stateVersion = "25.11";

        # Minimal bootable disk so all NixOS core assertions hold
        # (eval-only: nothing is built or booted here).
        fileSystems."/" = {
          device = "/dev/vda";
          fsType = "ext4";
        };
        boot.loader.grub.device = "/dev/vda";
      }
      ../modules
      {
        services.procurator.enable = true;

        services.procurator.control-plane = {
          enable = true;
          addr = "127.0.0.1:8080";
          hostname = "system-eval";
          workersAddr = [ { id = "worker-1"; addr = "127.0.0.1:9080"; } ];
        };

        services.procurator.worker = {
          enable = true;
          workerId = "worker-1";
          masterAddr = "127.0.0.1:8080";
          listenAddr = "0.0.0.0:9080";
          proxyJwtHs256Secret = "system-eval-secret";
        };

        services.procurator.cache.enable = true;
        services.procurator.ci.enable = true;
        services.procurator.repohub.enable = true;
        services.procurator.gitServer.enable = true;
      }
    ];
  };

  cfg = nixosEval.config;

  services = [
    "procurator-control-plane"
    "procurator-cache"
    "procurator-ci"
    "procurator-repohub"
    "procurator-worker"
  ];

  users = [
    "procurator" "procurator-cache" "procurator-ci"
    "procurator-repohub" "procurator-worker" "git"
  ];

  binaries = [
    { unit = "procurator-control-plane"; bin = "control_plane"; }
    { unit = "procurator-cache"; bin = "cache --config"; }
    { unit = "procurator-ci"; bin = "ci_service --config"; }
    { unit = "procurator-repohub"; bin = "repohub --config"; }
    { unit = "procurator-worker"; bin = "worker"; }
  ];

  assert' = msg: cond:
    if cond
    then true
    else builtins.throw "ASSERTION FAILED: ${msg}";

  # Forces the whole config (option types, merges, assertions consumers).
  assertionsClear =
    let failed = builtins.filter (a: !a.assertion) cfg.assertions;
    in
      if failed == []
      then true
      else builtins.throw ("ASSERTIONS FAILED: " + builtins.toJSON (map (a: a.message) failed));

in
assertionsClear
&& assert' "all five service units exist"
  (builtins.all (u: cfg.systemd.services ? ${u}) services)
&& assert' "ExecStart binaries match the implemented crates"
  (builtins.all
    (b: let execStart = cfg.systemd.services.${b.unit}.serviceConfig.ExecStart; in
      assert' "${b.unit}: ExecStart=${toString execStart}" (lib.hasInfix b.bin (toString execStart)))
    binaries)
&& assert' "service users exist" (builtins.all (u: cfg.users.users ? ${u}) users)
&& assert' "repohub joins git group"
  (lib.elem "git" cfg.users.users."procurator-repohub".extraGroups)
&& assert' "git-server hook installed" (cfg.environment.etc ? "git-server/post-receive")
&& assert' "cache storage dir tmpfile exists"
  (lib.any (r: builtins.match "d /var/lib/procurator-cache(/nar)? .*" r != null) cfg.systemd.tmpfiles.rules)
&& assert' "worker gets CAP_NET_ADMIN for TAP devices"
  (lib.elem "CAP_NET_ADMIN" cfg.systemd.services."procurator-worker".serviceConfig.AmbientCapabilities)
