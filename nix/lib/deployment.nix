# A repository's `procuratorDeployment` flake output: the file the CI reads
# after a passing run on the deploy branch and hands to the control plane
# (format version 1, validated by ci_service/src/domain/deploy.rs).
#
# Each VM is a NixOS system made with `diskVm` (direct kernel boot, raw root
# disk). The file references the kernel, initrd, toplevel and disk image, so
# building it builds them, and the CI pushes them to the cache with it.
#
#   procuratorDeployment = procurator.libs.${system}.deployment {
#     vms.web = { worker = "worker-1"; nixos = vm.vmConfig; cpus = 2; memoryMiB = 1024; };
#   };
{ pkgs, lib ? pkgs.lib }:
{ vms }:
let
  entry = id: vm:
    let
      config = vm.nixos.config;
      toplevel = config.system.build.toplevel;
    in
    {
      inherit id;
      inherit (vm) worker;
      kernel = "${config.boot.kernelPackages.kernel}/${config.system.boot.loader.kernelFile}";
      initramfs = "${config.system.build.initialRamdisk}/initrd";
      cmdline = lib.concatStringsSep " " (config.boot.kernelParams ++ [ "init=${toplevel}/init" ]);
      disk = "${config.system.build.rawImage}/nixos.img";
      cpus = vm.cpus or 1;
      memoryMiB = vm.memoryMiB or 1024;
    };
in
pkgs.writeText "procurator-deployment.json" (builtins.toJSON {
  version = 1;
  vms = lib.mapAttrsToList entry vms;
})
