# mkVmProfile — Pure validation and normalization of VM guest configuration.
#
# Returns a plain attrset (NOT a NixOS module, NOT an image — just data).
# This is the single source of truth for what goes inside a VM and how
# big it is. Both mkVmImage and evalCluster consume profiles.
#
# Usage:
#   mkVmProfile = import ./profile {};
#   profile = mkVmProfile {
#     hostname = "sandbox";
#     cpu = 2;
#     memoryMb = 1024;
#     packages = p: [ p.python3 ];
#     allowedDomains = [ "api.openai.com" ];
#   };
#
# All fields have sensible defaults. The only requirement is that cpu
# and memoryMb must be positive integers when provided.

{}: # No dependencies — this is pure data validation

# mkVmProfile function
{
  hostname ? "ch-vm",
  cpu ? 1,
  memoryMb ? 512,
  packages ? (_: []),
  entrypoint ? null,
  autoShutdown ? false,
  allowedDomains ? [],
  sshAuthorizedKeys ? [],
  files ? {},
}:

let
  # ── Validation ─────────────────────────────────────────────────────
  assertPositiveInt = name: val:
    if !(builtins.isInt val) then
      builtins.throw "mkVmProfile: ${name} must be an integer, got ${builtins.typeOf val}"
    else if val <= 0 then
      builtins.throw "mkVmProfile: ${name} must be positive, got ${toString val}"
    else
      true;

  assertString = name: val:
    if !(builtins.isString val) then
      builtins.throw "mkVmProfile: ${name} must be a string, got ${builtins.typeOf val}"
    else
      true;

  assertBool = name: val:
    if !(builtins.isBool val) then
      builtins.throw "mkVmProfile: ${name} must be a bool, got ${builtins.typeOf val}"
    else
      true;

  assertList = name: val:
    if !(builtins.isList val) then
      builtins.throw "mkVmProfile: ${name} must be a list, got ${builtins.typeOf val}"
    else
      true;

  assertFunction = name: val:
    if !(builtins.isFunction val) then
      builtins.throw "mkVmProfile: ${name} must be a function (pkgs -> [package]), got ${builtins.typeOf val}"
    else
      true;

  assertAttrs = name: val:
    if !(builtins.isAttrs val) then
      builtins.throw "mkVmProfile: ${name} must be an attrset, got ${builtins.typeOf val}"
    else
      true;

  assertNullOrString = name: val:
    if val != null && !(builtins.isString val) then
      builtins.throw "mkVmProfile: ${name} must be a string or null, got ${builtins.typeOf val}"
    else
      true;

  # Run all validations (short-circuit on first failure via assert)
  validated =
    assert assertString "hostname" hostname;
    assert assertPositiveInt "cpu" cpu;
    assert assertPositiveInt "memoryMb" memoryMb;
    assert assertFunction "packages" packages;
    assert assertNullOrString "entrypoint" entrypoint;
    assert assertBool "autoShutdown" autoShutdown;
    assert assertList "allowedDomains" allowedDomains;
    assert assertList "sshAuthorizedKeys" sshAuthorizedKeys;
    assert assertAttrs "files" files;
    true;

in
  # Return the normalized profile attrset
  assert validated;
  {
    inherit
      hostname
      cpu
      memoryMb
      packages
      entrypoint
      autoShutdown
      allowedDomains
      sshAuthorizedKeys
      files
      ;

    # Marker to identify this as a validated profile
    _type = "vmProfile";
  }
