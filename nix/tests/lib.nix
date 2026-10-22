# Shared helpers for the fast NixOS service-module eval tests.
# Pure eval, no builds: `writeText`/`formats.toml.generate` return strings
# tests can parse back, and package stubs are inert derivations.
{ lib }:
rec {
  # Inert derivation for `types.package` — only its out path is interpolated.
  pkgStub = { name, system }:
    derivation {
      name = "${name}-stub";
      builder = "/bin/sh";
      args = ["-c" ":"];
      inherit system;
    };

  # Minimal pkgs stub covering what the service modules touch at eval time.
  # `packages` names the stubbed crate binaries (e.g. ["worker" "cache"]).
  # `system` replaces builtins.currentSystem, which `nix flake check` forbids.
  mkPkgsStub = { packages, system ? "x86_64-linux" }:
    {
      writeText = _name: text: text;
      writeScript = _name: text: text;
      package = "pkg";
      runtimeShell = "/bin/sh";
      coreutils = "/bin";
      cloud-hypervisor = "/bin";
      # `""` so `${pkgs.git}/bin/git-shell` renders as `/bin/git-shell`.
      git = "";
      runtimeShellPackage = "/bin";
      # `formats.toml {}` serialises to JSON in tests so they can parse and
      # assert on the generated config without a TOML parser.
      formats = {
        toml = _: {
          generate = _name: value: builtins.toJSON value;
        };
      };
    }
    // builtins.listToAttrs (
      map (n: lib.nameValuePair n (pkgStub { name = n; inherit system; })) packages
    );

  # NixOS-core options the modules set that are out of scope for pure eval.
  stubSystemd = {
    lib,
    ...
  }: {
    options.systemd.services =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.unspecified;
      default = [];
    };
    options.users.users =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.users.groups =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.environment.etc =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.networking =
      lib.mkOption { type = lib.types.attrsOf lib.types.unspecified; default = {}; };
    options.assertions = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          assertion = lib.mkOption { type = lib.types.bool; };
          message = lib.mkOption { type = lib.types.str; };
        };
      });
      default = [];
    };
  };

  # Force assertion checking (evalModules alone does not run them).
  checkAssertions = evalResult:
    let
      failed = builtins.filter (a: !a.assertion) evalResult.config.assertions;
    in
      if failed == []
      then true
      else builtins.throw ("ASSERTIONS FAILED: " + builtins.toJSON (map (a: a.message) failed));

  # ── Assertion helpers ──────────────────────────────────────────────
  assert' = msg: cond:
    if cond
    then true
    else builtins.throw "ASSERTION FAILED: ${msg}";

  assertEq = name: actual: expected:
    assert' "${name}: expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}" (
      actual == expected
    );

  assertThrows = name: expr:
    assert' "${name}: expected evaluation to throw, but it succeeded"
      (!(builtins.tryEval expr).success);

  # Pull a JSON string back out of an ExecStart of the form
  # "<pkg>/bin/<bin-path> --config <json> ..." (the stub pkgs embed the
  # string context; discard it — test-only: JSON content carries no paths).
  # Intended for `writeText`-/`formats`-stub modules whose config file is
  # serialized with `builtins.toJSON` (or formats stub above).
  extractExecStartConfig = execStart: prefix:
    let start = builtins.stringLength prefix; in
    builtins.fromJSON (builtins.unsafeDiscardStringContext (builtins.substring start (builtins.stringLength execStart - start) execStart));
}
