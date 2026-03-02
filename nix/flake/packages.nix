{
  pkgs,
  workspaceRoot,
}:
let
  mkRustPackage = cargoDir:
    import ./mk-rust-package.nix {
      inherit pkgs workspaceRoot cargoDir;
    };
in {
  cache = mkRustPackage "cache";
  ci_service = mkRustPackage "ci_service";
  worker = mkRustPackage "worker";
  cli = mkRustPackage "cli";
}
