# Example: Running a Python script as a VM workload
#
# This shows how to use mkVmFromDrv to take a Python script,
# package it as a derivation, and run it inside an isolated VM.
#
# Usage:
#   cd flake-vmm/examples/python-workload
#   nix build .#vm-image    # build the VM image (~1.2GB)
#   nix build .#vm-spec     # inspect the VM specification (JSON)
#
# The VM will:
#   1. Boot NixOS with systemd
#   2. Run test.py automatically (via vm-workload.service)
#   3. Shut down when the script finishes
#
# To run it (from the cloud-hypervisor repo root):
#   nix run ./flake-vmm/examples/python-workload#run-vm
#
# Networking:
#   On the NixOS host, import ch-vmm.nixosModules.host and
#   declare the VM in ch-host.vms for bridge/TAP/domain filtering.

{
  description = "Python workload running in a Cloud Hypervisor VM";

  inputs = {
    ch-vmm.url = "path:../..";  # points to flake-vmm/
    nixpkgs.follows = "ch-vmm/nixpkgs";
  };

  outputs = { ch-vmm, nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      chLib = ch-vmm.lib.${system};

      # ── Package the Python script as a derivation ──────────────
      # This is the key step: turn your source code into something
      # Nix can put in the store and the VM can run.
      pythonWorkload = pkgs.stdenv.mkDerivation {
        pname = "python-workload";
        version = "0.1.0";

        # The script source — in a real project this would be
        # src = ./.; or fetched from a git repo
        src = pkgs.writeTextDir "test.py" (builtins.readFile ./test.py);

        buildInputs = [ pkgs.python3 ];

        installPhase = ''
          mkdir -p $out/bin $out/share/workload

          # Copy the Python source
          cp $src/test.py $out/share/workload/test.py

          # Create a wrapper script that sets up the right environment
          cat > $out/bin/python-workload <<'WRAPPER'
          #!/usr/bin/env bash
          set -e
          cd /root
          exec python3 ${placeholder "out"}/share/workload/test.py "$@"
          WRAPPER
          chmod +x $out/bin/python-workload
        '';

        meta.mainProgram = "python-workload";
      };

      # ── Build the VM image ─────────────────────────────────────
      # mkVmFromDrv:
      #   - Adds pythonWorkload + python3 to the image
      #   - Sets entrypoint to run it on boot
      #   - autoShutdown = true → VM powers off when script finishes
      #   - allowedDomains → only these domains reachable from the VM
      vm = chLib.mkVmFromDrv {
        drv = pythonWorkload;
        # entrypoint is auto-detected from meta.mainProgram → "python-workload"

        autoShutdown = true;
        hostname = "python-vm";

        # Extra Python packages available at runtime
        extraPackages = p: [ p.python3 ];

        # Only allow the VM to reach pypi (for example)
        allowedDomains = [ "pypi.org" "files.pythonhosted.org" ];

        # Inject config files into the image
        files = {
          "/etc/motd" = ''
            === Python Workload VM ===
            Script: test.py
            This VM auto-shuts down when the workload finishes.
          '';
        };
      };

      # ── VM runner script (for dev/testing) ─────────────────────
      vmRunner = pkgs.writeShellScriptBin "run-python-vm" ''
        #!/usr/bin/env bash
        set -e
        WORK_DIR="./vm-artifacts"
        mkdir -p "$WORK_DIR"

        echo "=== Python Workload VM ==="
        echo "Preparing ephemeral disk..."

        cp "${vm.image}" "$WORK_DIR/python-workload.raw"
        chmod 644 "$WORK_DIR/python-workload.raw"

        KERNEL="${vm.vmSpec.kernel}"
        INITRD="${vm.vmSpec.initrd}"

        [ -f "$WORK_DIR/py-bzImage" ] || cp "$KERNEL" "$WORK_DIR/py-bzImage"
        [ -f "$WORK_DIR/py-initrd" ]  || cp "$INITRD"  "$WORK_DIR/py-initrd"
        chmod 644 "$WORK_DIR/py-bzImage" "$WORK_DIR/py-initrd"

        CH="''${CH_BIN:-./target/release/cloud-hypervisor}"
        if [ ! -f "$CH" ]; then
          echo "cloud-hypervisor not found at $CH"
          echo "Set CH_BIN= or build with: cargo build --release"
          exit 1
        fi

        # Check for TAP device
        TAP_NAME="''${TAP:-chvm-pythonvm}"
        if ip link show "$TAP_NAME" &>/dev/null; then
          NET_ARGS="--net tap=$TAP_NAME"
        else
          echo "No TAP device '$TAP_NAME'. Running without networking."
          echo "(On NixOS host: enable ch-host with this VM in ch-host.vms)"
          NET_ARGS=""
        fi

        echo "Starting VM (will auto-shutdown when script finishes)..."
        echo ""

        "$CH" \
          --kernel "$WORK_DIR/py-bzImage" \
          --initramfs "$WORK_DIR/py-initrd" \
          --disk path="$WORK_DIR/python-workload.raw" \
          --cmdline "${vm.vmSpec.cmdline}" \
          --serial tty \
          --console off \
          --cpus boot=2 \
          --memory size=512M \
          $NET_ARGS \
          "$@"
      '';

    in {
      packages.${system} = {
        # The VM disk image — this is what the orchestrator deploys
        vm-image = vm.image;

        # The full VM spec (for programmatic consumption)
        vm-spec = pkgs.writeText "vm-spec.json" (builtins.toJSON {
          inherit (vm.vmSpec) hostname kernel initrd cmdline
                              allowedDomains entrypoint autoShutdown;
          image = toString vm.image;
        });

        # The Python derivation alone (for testing outside a VM)
        workload = pythonWorkload;

        # Runner script
        run-vm = vmRunner;

        default = vm.image;
      };

      apps.${system} = {
        run-vm = { type = "app"; program = "${vmRunner}/bin/run-python-vm"; };
        default = { type = "app"; program = "${vmRunner}/bin/run-python-vm"; };
      };

      # ── Host NixOS config snippet ─────────────────────────────
      # Shows what to add to your host's configuration.nix:
      #
      #   imports = [ ch-vmm.nixosModules.host ];
      #   ch-host.enable = true;
      #   ch-host.vms.python-workload = {
      #     allowedDomains = [ "pypi.org" "files.pythonhosted.org" ];
      #   };
    };
}
