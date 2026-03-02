{
  pkgs,
  workspaceRoot,
  cargoDir,
}:
let
  cargoPath = "${workspaceRoot}/${cargoDir}/Cargo.toml";
  cargoToml = builtins.fromTOML (builtins.readFile cargoPath);
  pname = cargoToml.package.name;
  version = cargoToml.package.version;
in
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
  }
