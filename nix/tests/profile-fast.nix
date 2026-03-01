# Fast test for mkVmProfile — pure eval, no NixOS build.
#
# Validates:
#   1. Default profile has correct field names and types
#   2. Custom values are preserved
#   3. _type marker is set to "vmProfile"
#   4. Validation rejects invalid inputs
#   5. All 10 fields present (9 user fields + _type)
#
# Run:
#   nix eval --json -f ./nix/tests/profile-fast.nix
#
# Returns true on success, throws on failure.

let
  mkVmProfile = import ../lib/profile {};

  # ── Assertion helpers ──────────────────────────────────────────────
  assert' = msg: cond:
    if cond then true
    else builtins.throw "ASSERTION FAILED: ${msg}";

  assertEq = name: actual: expected:
    assert' "${name}: expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}"
      (actual == expected);

  assertIsString = name: val:
    assert' "${name}: expected string, got ${builtins.typeOf val}"
      (builtins.isString val);

  assertIsInt = name: val:
    assert' "${name}: expected int, got ${builtins.typeOf val}"
      (builtins.isInt val);

  assertIsBool = name: val:
    assert' "${name}: expected bool, got ${builtins.typeOf val}"
      (builtins.isBool val);

  assertIsList = name: val:
    assert' "${name}: expected list, got ${builtins.typeOf val}"
      (builtins.isList val);

  assertIsFunction = name: val:
    assert' "${name}: expected lambda, got ${builtins.typeOf val}"
      (builtins.isFunction val);

  assertIsAttrs = name: val:
    assert' "${name}: expected set, got ${builtins.typeOf val}"
      (builtins.isAttrs val);

  assertThrows = name: expr:
    assert' "${name}: expected evaluation to throw, but it succeeded"
      (!(builtins.tryEval expr).success);

  # ── Expected fields (sorted) ───────────────────────────────────────
  expectedFields = [
    "_type"
    "allowedDomains"
    "autoShutdown"
    "cpu"
    "entrypoint"
    "files"
    "hostname"
    "memoryMb"
    "packages"
    "sshAuthorizedKeys"
  ];

  # ── 1. Default profile ─────────────────────────────────────────────
  defaultProfile = mkVmProfile {};

  defaultFieldNames = assertEq "default field names"
    (builtins.attrNames defaultProfile)
    expectedFields;

  defaultFieldCount = assertEq "default field count"
    (builtins.length (builtins.attrNames defaultProfile))
    (builtins.length expectedFields);

  # ── 2. Default types ───────────────────────────────────────────────
  defaultTypes =
    assertIsString "hostname type" defaultProfile.hostname
    && assertIsInt "cpu type" defaultProfile.cpu
    && assertIsInt "memoryMb type" defaultProfile.memoryMb
    && assertIsFunction "packages type" defaultProfile.packages
    && assertIsBool "autoShutdown type" defaultProfile.autoShutdown
    && assertIsList "allowedDomains type" defaultProfile.allowedDomains
    && assertIsList "sshAuthorizedKeys type" defaultProfile.sshAuthorizedKeys
    && assertIsAttrs "files type" defaultProfile.files
    && assertIsString "_type type" defaultProfile._type;

  # ── 3. Default values ──────────────────────────────────────────────
  defaultValues =
    assertEq "default hostname" defaultProfile.hostname "ch-vm"
    && assertEq "default cpu" defaultProfile.cpu 1
    && assertEq "default memoryMb" defaultProfile.memoryMb 512
    && assertEq "default entrypoint" defaultProfile.entrypoint null
    && assertEq "default autoShutdown" defaultProfile.autoShutdown false
    && assertEq "default allowedDomains" defaultProfile.allowedDomains []
    && assertEq "default sshAuthorizedKeys" defaultProfile.sshAuthorizedKeys []
    && assertEq "default files" defaultProfile.files {}
    && assertEq "default _type" defaultProfile._type "vmProfile";

  # ── 4. Custom values ───────────────────────────────────────────────
  customProfile = mkVmProfile {
    hostname = "sandbox";
    cpu = 4;
    memoryMb = 2048;
    packages = p: [ p ];  # dummy function
    entrypoint = "/bin/myapp";
    autoShutdown = true;
    allowedDomains = [ "api.openai.com" "github.com" ];
    sshAuthorizedKeys = [ "ssh-ed25519 AAAA..." ];
    files = { "/etc/config" = "hello"; };
  };

  customValues =
    assertEq "custom hostname" customProfile.hostname "sandbox"
    && assertEq "custom cpu" customProfile.cpu 4
    && assertEq "custom memoryMb" customProfile.memoryMb 2048
    && assertEq "custom entrypoint" customProfile.entrypoint "/bin/myapp"
    && assertEq "custom autoShutdown" customProfile.autoShutdown true
    && assertEq "custom allowedDomains" customProfile.allowedDomains [ "api.openai.com" "github.com" ]
    && assertEq "custom sshAuthorizedKeys" customProfile.sshAuthorizedKeys [ "ssh-ed25519 AAAA..." ]
    && assertEq "custom files" customProfile.files { "/etc/config" = "hello"; }
    && assertEq "custom _type" customProfile._type "vmProfile";

  # ── 5. Same field names for custom ─────────────────────────────────
  customFieldNames = assertEq "custom field names"
    (builtins.attrNames customProfile)
    expectedFields;

  # ── 6. Validation: bad cpu ─────────────────────────────────────────
  badCpuString = assertThrows "cpu as string"
    (mkVmProfile { cpu = "two"; });

  badCpuZero = assertThrows "cpu = 0"
    (mkVmProfile { cpu = 0; });

  badCpuNegative = assertThrows "cpu = -1"
    (mkVmProfile { cpu = -1; });

  # ── 7. Validation: bad memoryMb ────────────────────────────────────
  badMemString = assertThrows "memoryMb as string"
    (mkVmProfile { memoryMb = "512"; });

  badMemZero = assertThrows "memoryMb = 0"
    (mkVmProfile { memoryMb = 0; });

  # ── 8. Validation: bad types ───────────────────────────────────────
  badHostname = assertThrows "hostname as int"
    (mkVmProfile { hostname = 42; });

  badAutoShutdown = assertThrows "autoShutdown as string"
    (mkVmProfile { autoShutdown = "yes"; });

  badAllowedDomains = assertThrows "allowedDomains as string"
    (mkVmProfile { allowedDomains = "api.openai.com"; });

  badFiles = assertThrows "files as list"
    (mkVmProfile { files = []; });

  badPackages = assertThrows "packages as list"
    (mkVmProfile { packages = []; });

  # ── 9. Validation: entrypoint null vs string ───────────────────────
  nullEntrypoint = assertEq "null entrypoint"
    (mkVmProfile { entrypoint = null; }).entrypoint null;

  stringEntrypoint = assertEq "string entrypoint"
    (mkVmProfile { entrypoint = "/bin/foo"; }).entrypoint "/bin/foo";

  badEntrypoint = assertThrows "entrypoint as int"
    (mkVmProfile { entrypoint = 42; });

in
  defaultFieldNames
  && defaultFieldCount
  && defaultTypes
  && defaultValues
  && customValues
  && customFieldNames
  && badCpuString
  && badCpuZero
  && badCpuNegative
  && badMemString
  && badMemZero
  && badHostname
  && badAutoShutdown
  && badAllowedDomains
  && badFiles
  && badPackages
  && nullEntrypoint
  && stringEntrypoint
  && badEntrypoint
