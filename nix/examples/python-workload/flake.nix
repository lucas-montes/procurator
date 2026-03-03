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

End-to-end test for the procurator worker + python VM workload.

Environment variables:
  PCR_WORKER_ADDR   Worker address (default: 127.0.0.1:6000)
  PCR_VM_DIR        Base VM directory (default: /tmp/procurator/vms)
  PCR_TIMEOUT       Max seconds to wait for VM results (default: 120)

Flow:
  1) Check worker connectivity
  2) Create VM from vm-spec-json
  3) Wait for VM workload to complete (poll serial.log for result markers)
  4) Parse structured JSON results from serial log
  5) Verify workload PASS
  6) Delete VM
  7) Verify VM is removed from list
EOF
        exit 0
      fi

      addr="''${PCR_WORKER_ADDR:-127.0.0.1:6000}"
      vm_dir_base="''${PCR_VM_DIR:-/tmp/procurator/vms}"
      timeout_secs="''${PCR_TIMEOUT:-120}"
      spec_path="${vm.vmSpecJson}"

      info() { echo "[python-e2e] $*"; }
      fail() { echo "[python-e2e] FAIL: $*" >&2; exit 1; }

      # ── Step 1: Check worker connectivity ──────────────────────────
      info "step 1/7: checking worker connectivity at $addr"
      ${cli}/bin/pcr-test --addr "$addr" read >/dev/null \
        || fail "cannot reach worker at $addr"

      # ── Step 2: Create VM ──────────────────────────────────────────
      info "step 2/7: creating VM from $spec_path"
      create_output="$(${cli}/bin/pcr-test --addr "$addr" create-vm --spec-file "$spec_path" 2>&1)"
      echo "$create_output"

      vm_id="$(echo "$create_output" | ${pkgs.gnugrep}/bin/grep -oE '[0-9a-fA-F-]{36}' | head -n1 || true)"
      if [ -z "$vm_id" ]; then
        fail "could not parse VM ID from create output"
      fi
      info "VM created: $vm_id"

      serial_log="$vm_dir_base/$vm_id/serial.log"
      info "serial log: $serial_log"

      # ── Step 3: Wait for workload results ──────────────────────────
      info "step 3/7: waiting for workload results (timeout: ''${timeout_secs}s)"
      start_time=$(date +%s)
      result_json=""

      while true; do
        elapsed=$(( $(date +%s) - start_time ))
        if [ "$elapsed" -ge "$timeout_secs" ]; then
          info "serial.log contents (last 50 lines):"
          tail -n 50 "$serial_log" 2>/dev/null || true
          fail "timeout (''${timeout_secs}s) waiting for workload results"
        fi

        if [ -f "$serial_log" ]; then
          # Extract JSON between markers
          result_json="$(${pkgs.gawk}/bin/awk \
            '/---PCR_RESULT_START---/{found=1; next} /---PCR_RESULT_END---/{found=0} found{print}' \
            "$serial_log" | head -n1 || true)"
          if [ -n "$result_json" ]; then
            break
          fi
        fi

        sleep 2
      done

      elapsed=$(( $(date +%s) - start_time ))
      info "workload completed in ''${elapsed}s"

      # ── Step 4: Parse results ──────────────────────────────────────
      info "step 4/7: parsing workload results"
      echo "$result_json" | ${pkgs.jq}/bin/jq .

      status="$(echo "$result_json" | ${pkgs.jq}/bin/jq -r '.status')"
      summary="$(echo "$result_json" | ${pkgs.jq}/bin/jq -r '.summary')"

      # ── Step 5: Verify PASS ────────────────────────────────────────
      info "step 5/7: verifying result"
      if [ "$status" != "pass" ]; then
        info "workload errors:"
        echo "$result_json" | ${pkgs.jq}/bin/jq '.errors'
        fail "workload status is '$status' ($summary)"
      fi
      info "workload PASSED: $summary"

      # ── Step 6: Delete VM ──────────────────────────────────────────
      info "step 6/7: deleting VM $vm_id"
      ${cli}/bin/pcr-test --addr "$addr" delete-vm "$vm_id" \
        || fail "delete-vm failed for $vm_id"
      info "VM deleted"

      # ── Step 7: Verify deletion ────────────────────────────────────
      info "step 7/7: verifying VM removed from list"
      list_output="$(${cli}/bin/pcr-test --addr "$addr" list-vms 2>&1)"
      if echo "$list_output" | ${pkgs.gnugrep}/bin/grep -q "$vm_id"; then
        fail "VM $vm_id still appears in list after deletion"
      fi
      info "VM confirmed removed from list"

      echo ""
      info "=========================================="
      info "  E2E TEST PASSED"
      info "  VM: $vm_id"
      info "  Workload: $summary"
      info "  Total time: ''${elapsed}s"
      info "=========================================="
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
