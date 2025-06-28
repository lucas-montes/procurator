let
  pkgs = import <nixpkgs> {};
in
  pkgs.stdenv.mkDerivation {
    pname = "dummy";
    version = "0.1.0";
    src = ./dummy;
    buildInputs = [pkgs.gcc];
    buildPhase = ''
      gcc -o dummy main.c
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
