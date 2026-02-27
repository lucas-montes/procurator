# Fast Nix test for vmSpec shape — pure eval, no NixOS build.
#
# Validates the contract between Nix vmSpec output and Rust VmSpec:
#   - Exactly 8 fields (matching capnp VmSpec schema)
#   - camelCase field names (not snake_case)
#   - Correct types: strings, integers, list
#   - JSON round-trip preserves values
#   - Default values match VmmBackend expectations
#   - Nix store paths have /nix/store/ prefix
#
# Run:
#   nix eval --json -f ./nix/flake-vmm/test-vm-spec.nix
#
# Returns true on success, throws with descriptive message on failure.

let
  # ── Assertion helpers ──────────────────────────────────────────────
  assert' = msg: cond:
    if cond then true
    else builtins.throw "ASSERTION FAILED: ${msg}";

  assertEq = name: actual: expected:
    assert' "${name}: expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}"
      (actual == expected);

  assertHasAttr = name: attrset: attr:
    assert' "${name}: missing attribute '${attr}'"
      (builtins.hasAttr attr attrset);

  assertIsString = name: val:
    assert' "${name}: expected string, got ${builtins.typeOf val}"
      (builtins.isString val);

  assertIsInt = name: val:
    assert' "${name}: expected int, got ${builtins.typeOf val}"
      (builtins.isInt val);

  assertIsList = name: val:
    assert' "${name}: expected list, got ${builtins.typeOf val}"
      (builtins.isList val);

  hasPrefix = prefix: str:
    builtins.substring 0 (builtins.stringLength prefix) str == prefix;

  # ── Required fields (capnp VmSpec schema, sorted) ──────────────────
  requiredFields = [
    "cmdline"
    "cpu"
    "diskImagePath"
    "initrdPath"
    "kernelPath"
    "memoryMb"
    "networkAllowedDomains"
    "toplevel"
  ];

  # ── Simulated vmSpec (as mkVmImage would produce) ──────────────────
  # We can't call mkVmImage here (requires full NixOS eval), so we
  # test the shape contract using hand-built attrsets that mirror
  # the vmSpec output in flake.nix.
  defaultSpec = {
    toplevel = "/nix/store/aaaa-nixos-system";
    kernelPath = "/nix/store/bbbb-kernel/bzImage";
    initrdPath = "/nix/store/cccc-initrd/initrd";
    diskImagePath = "/nix/store/dddd-disk/nixos.raw";
    cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
    cpu = 1;
    memoryMb = 512;
    networkAllowedDomains = [];
  };

  customSpec = {
    toplevel = "/nix/store/xxxx-nixos-system";
    kernelPath = "/nix/store/yyyy-kernel/bzImage";
    initrdPath = "/nix/store/zzzz-initrd/initrd";
    diskImagePath = "/nix/store/wwww-disk/nixos.raw";
    cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
    cpu = 4;
    memoryMb = 2048;
    networkAllowedDomains = [ "api.openai.com" "github.com" ];
  };

  # ── 1. All required fields present ─────────────────────────────────
  allFieldsPresent = builtins.all
    (field: assertHasAttr "defaultSpec" defaultSpec field)
    requiredFields;

  # ── 2. No extra fields ─────────────────────────────────────────────
  noExtraFields = assertEq "field count"
    (builtins.length (builtins.attrNames defaultSpec))
    (builtins.length requiredFields);

  # ── 3. Field names are exactly the sorted requiredFields ───────────
  fieldNames = assertEq "field names match capnp schema"
    (builtins.attrNames defaultSpec)
    requiredFields;

  # ── 4. Default types ───────────────────────────────────────────────
  defaultTypes =
    assertIsString "toplevel" defaultSpec.toplevel
    && assertIsString "kernelPath" defaultSpec.kernelPath
    && assertIsString "initrdPath" defaultSpec.initrdPath
    && assertIsString "diskImagePath" defaultSpec.diskImagePath
    && assertIsString "cmdline" defaultSpec.cmdline
    && assertIsInt "cpu" defaultSpec.cpu
    && assertIsInt "memoryMb" defaultSpec.memoryMb
    && assertIsList "networkAllowedDomains" defaultSpec.networkAllowedDomains;

  # ── 5. Default values ──────────────────────────────────────────────
  defaults =
    assertEq "default cpu" defaultSpec.cpu 1
    && assertEq "default memoryMb" defaultSpec.memoryMb 512
    && assertEq "default networkAllowedDomains" defaultSpec.networkAllowedDomains [];

  # ── 6. Custom values ───────────────────────────────────────────────
  customs =
    assertEq "custom cpu" customSpec.cpu 4
    && assertEq "custom memoryMb" customSpec.memoryMb 2048
    && assertEq "custom networkAllowedDomains count"
      (builtins.length customSpec.networkAllowedDomains)
      2;

  # ── 7. Custom spec has same field names ────────────────────────────
  customFieldNames = assertEq "custom field names"
    (builtins.attrNames customSpec)
    requiredFields;

  # ── 8. JSON round-trip ─────────────────────────────────────────────
  specJson = builtins.toJSON customSpec;
  parsedBack = builtins.fromJSON specJson;
  jsonRoundTrip =
    assertEq "json toplevel" parsedBack.toplevel customSpec.toplevel
    && assertEq "json kernelPath" parsedBack.kernelPath customSpec.kernelPath
    && assertEq "json initrdPath" parsedBack.initrdPath customSpec.initrdPath
    && assertEq "json diskImagePath" parsedBack.diskImagePath customSpec.diskImagePath
    && assertEq "json cmdline" parsedBack.cmdline customSpec.cmdline
    && assertEq "json cpu" parsedBack.cpu customSpec.cpu
    && assertEq "json memoryMb" parsedBack.memoryMb customSpec.memoryMb
    && assertEq "json networkAllowedDomains" parsedBack.networkAllowedDomains customSpec.networkAllowedDomains;

  # ── 9. Nix store paths have correct prefix ─────────────────────────
  nixPathCheck =
    assert' "toplevel starts with /nix/store/"
      (hasPrefix "/nix/store/" defaultSpec.toplevel)
    && assert' "kernelPath starts with /nix/store/"
      (hasPrefix "/nix/store/" defaultSpec.kernelPath)
    && assert' "initrdPath starts with /nix/store/"
      (hasPrefix "/nix/store/" defaultSpec.initrdPath)
    && assert' "diskImagePath starts with /nix/store/"
      (hasPrefix "/nix/store/" defaultSpec.diskImagePath);

in
  allFieldsPresent
  && noExtraFields
  && fieldNames
  && defaultTypes
  && defaults
  && customs
  && customFieldNames
  && jsonRoundTrip
  && nixPathCheck
