# evalCluster — Validate cluster topology and link profiles to VMs.
#
# Takes a cluster spec with VM definitions referencing profiles (from
# mkVmProfile). Validates the topology and returns a structured attrset
# consumable by the control plane for scheduling.
#
# Usage:
#   evalCluster = import ./cluster {};
#   cluster = evalCluster {
#     vms = {
#       sandbox-1 = {
#         vmProfile = myProfile;
#         deployment.addr = "10.0.0.1";
#       };
#     };
#   };

{}:

# evalCluster function
{
  vms ? {},
}:

let
  lib = builtins;

  # ── Validate each VM entry ──────────────────────────────────────────
  validateVm = name: vm:
    let
      hasProfile = vm ? vmProfile;
      profile =
        if !hasProfile then
          builtins.throw "evalCluster: VM '${name}' is missing required field 'vmProfile'"
        else if (vm.vmProfile._type or null) != "vmProfile" then
          builtins.throw "evalCluster: VM '${name}'.vmProfile must be a validated profile (from mkVmProfile)"
        else
          vm.vmProfile;

      hasDeployment = vm ? deployment;
      deployment =
        if !hasDeployment then
          builtins.throw "evalCluster: VM '${name}' is missing required field 'deployment'"
        else
          vm.deployment;

      addr =
        if !(deployment ? addr) then
          builtins.throw "evalCluster: VM '${name}'.deployment.addr is required"
        else
          deployment.addr;

    in {
      vmProfile = profile;
      deployment = {
        inherit addr;
        backend = deployment.backend or "cloud-hypervisor";
        sshUser = deployment.sshUser or "root";
        sshPort = deployment.sshPort or 22;
        healthChecks = deployment.healthChecks or [];
        autoRollback = deployment.autoRollback or true;
      };
      role = vm.role or "worker";
      labels = vm.labels or [];
      replicas = vm.replicas or 1;

      # Derived from profile — for scheduling decisions
      cpu = profile.cpu;
      memoryMb = profile.memoryMb;
    };

  validatedVms = builtins.mapAttrs validateVm vms;

in {
  vms = validatedVms;

  # Summary stats for the control plane
  vmCount = builtins.length (builtins.attrNames validatedVms);
  totalCpu = builtins.foldl' (acc: vm: acc + vm.cpu) 0 (builtins.attrValues validatedVms);
  totalMemoryMb = builtins.foldl' (acc: vm: acc + vm.memoryMb) 0 (builtins.attrValues validatedVms);
}
