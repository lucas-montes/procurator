# Integration test flake — builds a real VM image with test.py as workload.
#
# This is the "slow" test: full NixOS eval + disk image build (~30-60s).
# It validates the complete pipeline: mkVmProfile → mkVmImage → vmSpec.
#
# Run:
#   nix flake check ./nix/tests/integration
#   nix build ./nix/tests/integration#checks.x86_64-linux.vm-spec-contract
#
# The check builds a real VM with the test.py workload, then validates
# that the vmSpec JSON matches the capnp schema with real /nix/store paths.
{
  description = "Procurator VM integration test";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs { inherit system; };

        # Import the procurator libs
        procuratorLib = import ../../lib { inherit pkgs nixpkgs system; };

        # The test workload script
        testWorkload = pkgs.writers.writePython3 "integration-test" {} (builtins.readFile ./test.py);

        # Create a profile for the test VM
        testProfile = procuratorLib.mkVmProfile {
          hostname = "integration-test";
          cpu = 2;
          memoryMb = 1024;
          packages = p: [ p.python3 testWorkload ];
          entrypoint = "${testWorkload}";
          autoShutdown = true;
          allowedDomains = [ "api.openai.com" "github.com" ];
          sshAuthorizedKeys = [];
          files = {
            "/etc/procurator/test-marker" = "integration-test-v1";
          };
        };

        # Build the VM image
        testVm = procuratorLib.mkVmImage {
          profile = testProfile;
        };

        spec = testVm.vmSpec;
        expectedFields = [ "cmdline" "cpu" "diskImagePath" "initrdPath" "kernelPath" "memoryMb" "networkAllowedDomains" "toplevel" ];
        specJson = builtins.toJSON spec;

      in {
        # ── Checks ─────────────────────────────────────────────────
        checks = {
          # Validates vmSpec shape from a real mkVmImage call.
          # This triggers full NixOS eval + disk image derivation.
          vm-spec-contract = pkgs.runCommand "check-vm-spec-contract" {
            passAsFile = [ "specContent" ];
            specContent = specJson;
            nativeBuildInputs = [ pkgs.jq ];
          } ''
            set -euo pipefail

            echo "=== vmSpec contract test (full NixOS eval) ==="
            echo ""

            # 1. Parse JSON
            echo "1. Parsing vmSpec JSON..."
            jq . < "$specContentPath" > /dev/null
            echo "   OK: valid JSON"

            # 2. Exactly 8 fields
            echo "2. Checking field count..."
            count=$(jq 'keys | length' < "$specContentPath")
            if [ "$count" != "8" ]; then
              echo "   FAIL: expected 8 fields, got $count"
              jq 'keys' < "$specContentPath"
              exit 1
            fi
            echo "   OK: 8 fields"

            # 3. Required fields present
            echo "3. Checking required fields..."
            for field in ${builtins.concatStringsSep " " expectedFields}; do
              if ! jq -e ".$field" < "$specContentPath" > /dev/null 2>&1; then
                echo "   FAIL: missing field '$field'"
                exit 1
              fi
            done
            echo "   OK: all required fields present"

            # 4. String fields are non-empty
            echo "4. Checking string field types..."
            for field in toplevel kernelPath initrdPath diskImagePath cmdline; do
              val=$(jq -r ".$field" < "$specContentPath")
              if [ -z "$val" ] || [ "$val" = "null" ]; then
                echo "   FAIL: '$field' is empty or null"
                exit 1
              fi
            done
            echo "   OK: all string fields non-empty"

            # 5. Integer fields are numbers
            echo "5. Checking integer fields..."
            for field in cpu memoryMb; do
              typ=$(jq -r ".$field | type" < "$specContentPath")
              if [ "$typ" != "number" ]; then
                echo "   FAIL: '$field' is $typ, expected number"
                exit 1
              fi
            done
            echo "   OK: cpu and memoryMb are numbers"

            # 6. Custom values propagated
            echo "6. Checking custom values..."
            cpu=$(jq '.cpu' < "$specContentPath")
            mem=$(jq '.memoryMb' < "$specContentPath")
            domains=$(jq '.networkAllowedDomains | length' < "$specContentPath")
            if [ "$cpu" != "2" ]; then
              echo "   FAIL: cpu expected 2, got $cpu"
              exit 1
            fi
            if [ "$mem" != "1024" ]; then
              echo "   FAIL: memoryMb expected 1024, got $mem"
              exit 1
            fi
            if [ "$domains" != "2" ]; then
              echo "   FAIL: networkAllowedDomains expected 2 entries, got $domains"
              exit 1
            fi
            echo "   OK: cpu=2, memoryMb=1024, 2 domains"

            # 7. Store paths have /nix/store/ prefix
            echo "7. Checking Nix store path format..."
            for field in toplevel kernelPath initrdPath diskImagePath; do
              val=$(jq -r ".$field" < "$specContentPath")
              case "$val" in
                /nix/store/*) ;;
                *)
                  echo "   FAIL: '$field' = '$val' (not a /nix/store/ path)"
                  exit 1
                  ;;
              esac
            done
            echo "   OK: all paths are /nix/store/ paths"

            # 8. Domain list content
            echo "8. Checking domain list..."
            first=$(jq -r '.networkAllowedDomains[0]' < "$specContentPath")
            second=$(jq -r '.networkAllowedDomains[1]' < "$specContentPath")
            if [ "$first" != "api.openai.com" ]; then
              echo "   FAIL: first domain expected 'api.openai.com', got '$first'"
              exit 1
            fi
            if [ "$second" != "github.com" ]; then
              echo "   FAIL: second domain expected 'github.com', got '$second'"
              exit 1
            fi
            echo "   OK: domains match"

            echo ""
            echo "=== All vmSpec contract tests passed ==="
            echo ""
            echo "vmSpec JSON:"
            jq . < "$specContentPath"

            mkdir -p $out
            cp "$specContentPath" $out/vm-spec.json
            echo "PASS" > $out/result
          '';
        };

        # ── Packages ─────────────────────────────────────────────────
        packages = {
          # The test VM image
          test-image = testVm.image;

          # The test VM spec JSON
          test-vm-spec = testVm.vmSpecJson;

          # The test workload script (for manual inspection)
          test-workload = testWorkload;

          default = testVm.image;
        };
      }
    );
}
