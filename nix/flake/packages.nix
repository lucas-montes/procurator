{
  pkgs,
  workspaceRoot,
  naersk ? null,
}:
let
  naerskLib = if naersk != null then pkgs.callPackage naersk {} else null;

  mkRustPackage = cargoDir: let
    cargoPath = "${workspaceRoot}/${cargoDir}/Cargo.toml";
    cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
    pname = cargoToml.package.name;
    version = cargoToml.package.version;
  in
    if naerskLib != null then
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
      }
    else
      pkgs.rustPlatform.buildRustPackage {
        inherit pname version;
        src = workspaceRoot;

        cargoLock = {
          lockFile = "${workspaceRoot}/Cargo.lock";
        };

        cargoBuildFlags = [
          "-p"
          pname
        ];
        cargoInstallFlags = [
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
}
