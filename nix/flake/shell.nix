{
  pkgs,
  rust-bin-custom,
  pcr-test-wrapper,
}:
with pkgs;
  mkShell {
    buildInputs = [
      cargo-watch
      pkg-config
      rust-bin-custom
      capnproto
      pkgs."cloud-hypervisor"
      pcr-test-wrapper
      openapi-generator-cli
    ];

    shellHook = ''
      export CARGO_MANIFEST_DIR="$PWD"
    '';
  }
