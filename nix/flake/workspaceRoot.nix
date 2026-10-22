# Shared source filter for the flake's Rust workspace.
# Excludes git internals, build dirs, and scratch files so SDL packages
# rebuild when relevant sources change but not on local scratch churn.
# Used both by the per-system package set and the exported overlay.
{ pkgs, src }:
pkgs.lib.cleanSourceWith {
  inherit src;
  filter =
    path: _type:
    let
      root = toString src;
      pathStr = toString path;
      relPath = pkgs.lib.removePrefix "${root}/" pathStr;
    in
    !(
      pkgs.lib.hasPrefix ".git/" relPath
      || pkgs.lib.hasPrefix "target/" relPath
      || pkgs.lib.hasPrefix ".direnv/" relPath
      || pkgs.lib.hasPrefix "result/" relPath
      || relPath == "result"
      || pkgs.lib.hasPrefix "tmp/" relPath
    );
}
