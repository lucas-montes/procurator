{
  pkgs,
  rust-bin-custom,
  pcr-test-wrapper,
}:
  pkgs.mkShell {
    name = "procurator";
    buildInputs = [
      pkgs.cargo-watch
      pkgs.pkg-config
      rust-bin-custom
      pkgs.capnproto
      pkgs.cloud-hypervisor
      pcr-test-wrapper
      pkgs.openapi-generator-cli
    ];

    shellHook = ''
      export CARGO_MANIFEST_DIR="$PWD"
    '';
  }
