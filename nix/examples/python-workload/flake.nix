{
  description = "Python workload example for procurator worker";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    procurator.url = "path:../..";
  };

  outputs = {
    self,
    nixpkgs,
    procurator,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};
    pLib = procurator.lib.${system};

    profile = pLib.mkVmProfile {
      hostname = "python-vm";
      cpu = 2;
      memoryMb = 1024;
      autoShutdown = true;
      allowedDomains = ["pypi.org" "files.pythonhosted.org"];
      packages = p: [p.python3];
      entrypoint = "python3 /opt/workload/test.py";
      files = {
        "/opt/workload/test.py" = builtins.readFile ./test.py;
      };
    };

    vm = pLib.mkVmImage {inherit profile;};

    cli = procurator.packages.${system}.cli;

    buildVmSpecPath = pkgs.writeShellScriptBin "build-python-vm-spec" ''
      set -euo pipefail
      echo "${vm.vmSpecJson}"
    '';

    runWorkerE2e = pkgs.writeShellScriptBin "run-python-worker-e2e" ''
      set -euo pipefail

      if [[ "''${1:-}" == "--help" || "''${1:-}" == "-h" ]]; then
        cat <<'EOF'
Usage: nix run ./nix/examples/python-workload#worker-e2e

Environment variables:
  PCR_WORKER_ADDR   Worker address (default: 127.0.0.1:6000)
  PCR_KEEP_VM       Set to 1 to keep VM instead of deleting

Flow:
  1) Check worker connectivity
  2) Create VM from vm-spec-json
  3) List VMs
  4) Delete VM (unless PCR_KEEP_VM=1)
EOF
        exit 0
      fi

      addr="''${PCR_WORKER_ADDR:-127.0.0.1:6000}"
      keep_vm="''${PCR_KEEP_VM:-0}"
      spec_path="${vm.vmSpecJson}"

      info() {
        echo "[python-e2e] $*"
      }

      info "checking worker connectivity at $addr"
      ${cli}/bin/pcr-test --addr "$addr" read >/dev/null

      info "creating vm from $spec_path"
      create_output="$(${cli}/bin/pcr-test --addr "$addr" create-vm --spec-file "$spec_path")"
      echo "$create_output"

      vm_id="$(echo "$create_output" | ${pkgs.gnugrep}/bin/grep -Eo '[0-9a-fA-F-]{36}' | head -n1 || true)"
      if [ -z "$vm_id" ]; then
        echo "failed to parse VM id from output" >&2
        exit 1
      fi

      cleanup_vm() {
        if [ "$keep_vm" = "1" ]; then
          info "keeping vm $vm_id (PCR_KEEP_VM=1)"
          return
        fi
        info "deleting vm $vm_id"
        ${cli}/bin/pcr-test --addr "$addr" delete-vm "$vm_id"
        info "final vm list"
        ${cli}/bin/pcr-test --addr "$addr" list-vms
      }

      trap cleanup_vm EXIT

      info "listing vms after create"
      ${cli}/bin/pcr-test --addr "$addr" list-vms
      info "done"
    '';
  in {
    packages.${system} = {
      vm-image = vm.image;
      vm-spec-json = vm.vmSpecJson;

      # Alias kept for docs and compatibility.
      vm-spec = vm.vmSpecJson;

      build-vm-spec-path = buildVmSpecPath;
      worker-e2e = runWorkerE2e;

      default = vm.image;
    };

    apps.${system} = {
      build-vm-spec-path = {
        type = "app";
        program = "${buildVmSpecPath}/bin/build-python-vm-spec";
      };
      worker-e2e = {
        type = "app";
        program = "${runWorkerE2e}/bin/run-python-worker-e2e";
      };
      default = {
        type = "app";
        program = "${runWorkerE2e}/bin/run-python-worker-e2e";
      };
    };
  };
}
