# This is just a dummy example for an app to have a second source
let
  pkgs = import <nixpkgs> {};
in
  pkgs.stdenv.mkDerivation {
    pname = "dummy";
    version = "0.1.0";
    src = ./;
    buildInputs = [pkgs.gcc];
    buildPhase = ''
      gcc -o dummy main.c
    '';
    doCheck = true;
    checkPhase = ''
          # Run the tests
          ./test_dummy.sh
        '';
    installPhase = ''
      mkdir -p $out/bin
      cp dummy $out/bin/
    '';
    meta = with pkgs.lib; {
      description = "A dummy C executable for testing";
      license = licenses.mit;
    };
  }
