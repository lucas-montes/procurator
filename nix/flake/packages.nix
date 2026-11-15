{
  pkgs,
  workspaceRoot,
  naersk,
}: let
  naerskLib = pkgs.callPackage naersk {};

  mkRustPackage = cargoDir: let
    cargoPath = "${workspaceRoot}/${cargoDir}/Cargo.toml";
    cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
    pname = cargoToml.package.name;
    inherit (cargoToml.package) version;
  in
    naerskLib.buildPackage {
      inherit pname version;
      src = workspaceRoot;

      cargoBuildOptions = opts:
        opts
        ++ [
          "-p"
          pname
        ];

      cargoTestOptions = opts:
        opts
        ++ [
          "-p"
          pname
        ];

      nativeBuildInputs = [
        pkgs.pkg-config
        pkgs.capnproto
      ];

      buildInputs = [pkgs.openssl];
      doCheck = false;
    };
in {
  cache = mkRustPackage "cache";
  ci_service = mkRustPackage "ci_service";
  worker = mkRustPackage "worker";
  cli = mkRustPackage "cli";
  control_plane = mkRustPackage "control_plane";
  console = mkRustPackage "console";
  repohub = mkRustPackage "repohub";
}
