# Fast test for `libs.<system>.deployment` — pure eval, no build.
#
# Validates that the deployment file lists each VM with the boot files of
# its NixOS system, in the format the CI reads (version 1).
#
# Run:
#   nix eval -f ./nix/tests/deployment-fast.nix --apply "f: f {}"
#
# Returns true on success, throws on failure.

{ lib ? (import <nixpkgs/lib>) }:
let
  helpers = import ./lib.nix { inherit lib; };
  inherit (helpers) assertEq;

  # Why: writeText only needs to hand the text back here.
  pkgsStub = {
    inherit lib;
    writeText = name: text: { inherit name text; };
  };
  deployment = import ../lib/deployment.nix { pkgs = pkgsStub; };

  # The parts of a `diskVm` NixOS system the file reads.
  nixos = {
    config = {
      boot.kernelPackages.kernel = "/nix/store/k-linux";
      system.boot.loader.kernelFile = "bzImage";
      system.build.initialRamdisk = "/nix/store/i-initrd";
      system.build.toplevel = "/nix/store/t-system";
      system.build.rawImage = "/nix/store/d-image";
      boot.kernelParams = [ "console=ttyS0" "root=/dev/vda" ];
    };
  };

  built = deployment {
    vms = {
      web = { worker = "worker-1"; inherit nixos; cpus = 2; memoryMiB = 2048; };
      db = { worker = "worker-2"; inherit nixos; };
    };
  };
  file = builtins.fromJSON built.text;
in
assert assertEq "name" built.name "procurator-deployment.json";
assert assertEq "file" file {
  version = 1;
  vms = [
    {
      id = "db";
      worker = "worker-2";
      kernel = "/nix/store/k-linux/bzImage";
      initramfs = "/nix/store/i-initrd/initrd";
      cmdline = "console=ttyS0 root=/dev/vda init=/nix/store/t-system/init";
      disk = "/nix/store/d-image/nixos.img";
      cpus = 1;
      memoryMiB = 1024;
    }
    {
      id = "web";
      worker = "worker-1";
      kernel = "/nix/store/k-linux/bzImage";
      initramfs = "/nix/store/i-initrd/initrd";
      cmdline = "console=ttyS0 root=/dev/vda init=/nix/store/t-system/init";
      disk = "/nix/store/d-image/nixos.img";
      cpus = 2;
      memoryMiB = 2048;
    }
  ];
};
true
