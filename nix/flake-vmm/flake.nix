# flake.nix — Cloud Hypervisor VM Builder
#
# A Nix flake that builds everything needed to run sandboxed NixOS VMs
# inside Cloud Hypervisor:
#
#   1. A stock NixOS kernel (bzImage for traditional boot)
#   2. Minimal NixOS ext4 disk images (systemd, SSH, configurable packages)
#   3. mkVmFromDrv — inject any derivation as a workload, auto-run on boot
#   4. host-module.nix — NixOS module for host networking (bridge, TAP,
#      NAT, dnsmasq, nftables domain allowlisting)
#   5. vm-module.nix — NixOS module for guest configuration
#
# ── Quick start ──────────────────────────────────────────────────────
#
#   # Build the default image (minimal NixOS + SSH):
#   nix build .#image
#
#   # Run the VM (dev mode, builds CH from source):
#   nix run .#vm
#
#   # Run the workload demo (auto-shuts down when done):
#   nix run .#workload-demo
#
# ── Host setup (NixOS) ──────────────────────────────────────────────
#
#   # In your host's flake.nix:
#   {
#     inputs.ch-vmm.url = "path:./flake-vmm";
#
#     outputs = { ch-vmm, nixpkgs, ... }: {
#       nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
#         modules = [
#           ch-vmm.nixosModules.host
#           {
#             ch-host.enable = true;
#             ch-host.vms = {
#               sandbox = {
#                 allowedDomains = [ "api.openai.com" "github.com" ];
#               };
#               ci-runner = {};  # full internet
#             };
#           }
#         ];
#       };
#     };
#   }
#
# ── Library usage (VM images) ───────────────────────────────────────
#
#   outputs = { ch-vmm, ... }: {
#     packages.x86_64-linux.my-vm =
#       (ch-vmm.lib.x86_64-linux.mkVmImage {
#         extraPackages = p: [ p.python3 p.htop ];
#         sshAuthorizedKeys = [ "ssh-ed25519 AAAA..." ];
#         hostname = "my-sandbox";
#         allowedDomains = [ "api.openai.com" ];
#       }).image;
#
#     # Or: run any derivation as a VM workload (gitops pattern):
#     packages.x86_64-linux.my-workload =
#       (ch-vmm.lib.x86_64-linux.mkVmFromDrv {
#         drv = myApp;
#         autoShutdown = true;
#         allowedDomains = [ "pypi.org" ];
#       }).image;
#   };
#
{
  description = "Cloud Hypervisor VM Builder — minimal NixOS images for sandboxed VMs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
    ...
  }:
    # ── System-independent outputs ───────────────────────────────────
    {
      # NixOS modules for host and guest configuration.
      # These are not per-system — they work on any NixOS.
      nixosModules = {
        # Host module: bridge, TAP, NAT, dnsmasq, nftables domain allowlisting
        # Import into your NixOS host config and set ch-host.enable = true.
        host = import ./host-module.nix;

        # Guest module: systemd, SSH, serial, virtio, workload entrypoint
        # (Usually you don't import this directly — mkVmImage does it for you.)
        guest = import ./vm-module.nix;
      };
    }
    //
    # ── Per-system outputs ───────────────────────────────────────────
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        lib = pkgs.lib;

        # ── Rust toolchain ─────────────────────────────────────────
        # Pinned stable Rust for building cloud-hypervisor from source.
        rust-bin-custom = pkgs.rust-bin.stable."1.88.0".default.override {
          extensions = ["rust-src" "rust-analyzer"];
        };

        # ── Kernel ─────────────────────────────────────────────────
        # Stock NixOS 6.6 LTS kernel. It already includes:
        #   - All virtio drivers (as modules, loaded by initrd)
        #   - ext4, serial console, PVH boot support
        #   - KVM guest optimizations
        #
        # We don't need a custom kernel config — the NixOS default
        # works perfectly with Cloud Hypervisor. The VM module
        # (vm-module.nix) ensures the right modules are in the initrd.
        #
        # Available boot formats:
        #   ${vmKernel}/bzImage  — compressed kernel (traditional boot)
        #   ${vmKernel}/vmlinux  — ELF binary (PVH direct boot, faster)
        vmKernel = pkgs.linux_6_6;

        # ── mkVmImage: the core library function ───────────────────
        # Builds a NixOS ext4 raw disk image for Cloud Hypervisor.
        #
        # Arguments:
        #   extraPackages    - function (pkgs -> [package]) of extra packages
        #   sshAuthorizedKeys - list of SSH public key strings
        #   hostname         - VM hostname (default: "ch-vm")
        #   diskSize         - disk image size (default: "auto")
        #   additionalSpace  - free space beyond the NixOS closure (default: "256M")
        #   entrypoint       - shell command to run as workload on boot (null = none)
        #   autoShutdown     - power off VM when entrypoint exits (default: false)
        #   files            - attrset of {path = content} to inject into the image
        #   allowedDomains   - list of domain names the VM can reach ([] = all)
        #
        # Returns: an attrset with:
        #   - image: the ext4 disk image derivation
        #   - nixos: the full NixOS system (for kernel/initrd access)
        #   - toplevel: the system.build.toplevel derivation
        #   - networkSetupScript: host-side script with baked-in domain allowlist
        #   - vmSpec: all metadata needed to launch (kernel, initrd, image, cmdline, etc.)
        mkVmImage = {
          extraPackages ? (_: []),
          sshAuthorizedKeys ? [],
          hostname ? "ch-vm",
          diskSize ? "auto",
          additionalSpace ? "256M",
          entrypoint ? null,
          autoShutdown ? false,
          files ? {},
          allowedDomains ? [],
        }: let
          # Build a NixOS system with our VM module + user overrides
          nixos = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              # Our base VM module (systemd, SSH, serial, virtio, minimal)
              ./vm-module.nix

              # Use our chosen kernel
              { boot.kernelPackages = pkgs.linuxPackagesFor vmKernel; }

              # User-provided configuration
              ({ pkgs, ... }: {
                ch-vm = {
                  inherit hostname sshAuthorizedKeys;
                  inherit entrypoint autoShutdown files allowedDomains;
                  extraPackages = extraPackages pkgs;
                };
              })

              # ── First-boot setup (sd-image pattern) ──────────────
              # Since we don't run nixos-install (no QEMU at build time),
              # we register the Nix store and create the system profile
              # on the very first boot. This is the same pattern used by
              # sd-image.nix, lxc-container.nix, and ISO images.
              ({ config, pkgs, ... }: {
                boot.postBootCommands = ''
                  # On first boot, register the Nix store contents and
                  # set up the system profile. The marker file is placed
                  # by populateImageCommands and removed after registration.
                  if [ -f /nix-path-registration ]; then
                    set -euo pipefail
                    ${pkgs.nix}/bin/nix-store --load-db < /nix-path-registration

                    # nixos-rebuild requires a "system" profile and /etc/NIXOS
                    touch /etc/NIXOS
                    ${pkgs.nix}/bin/nix-env -p /nix/var/nix/profiles/system --set /run/current-system

                    rm -f /nix-path-registration
                  fi
                '';
              })
            ];
          };

          toplevel = nixos.config.system.build.toplevel;

          # Build the disk image using make-ext4-fs.nix (NO QEMU!).
          #
          # This is the sd-image pattern: instead of running nixos-install
          # inside a QEMU VM, we directly populate the ext4 image:
          #   1. storePaths → copies the entire Nix store closure
          #   2. populateImageCommands → creates the minimal FS skeleton
          #   3. boot.postBootCommands → finishes setup on first boot
          #
          # The key trick: we create /sbin/init → toplevel/init so that
          # the kernel can find init, and then NixOS's stage-2 init +
          # activation scripts create the full /etc, /var, users, etc.
          image = pkgs.callPackage "${nixpkgs}/nixos/lib/make-ext4-fs.nix" {
            compressImage = false;
            volumeLabel = "nixos";  # Matches fileSystems in vm-module.nix
            storePaths = [ toplevel ];
            populateImageCommands = ''
              # ── Minimal filesystem skeleton ──────────────────────
              # NixOS stage-2 init and activation scripts will create
              # the rest (/etc, /var, users, etc.) on first boot.

              # /sbin/init is what the kernel runs (cmdline: init=/sbin/init
              # is the default). Point it at the NixOS toplevel's init.
              mkdir -p ./files/sbin
              ln -s ${toplevel}/init ./files/sbin/init

              # /etc/NIXOS tells NixOS activation that this is a NixOS system
              mkdir -p ./files/etc
              touch ./files/etc/NIXOS

              # Pre-create directories that init/activation expect to exist
              mkdir -p ./files/var/log
              mkdir -p ./files/var/lib
              mkdir -p ./files/proc
              mkdir -p ./files/sys
              mkdir -p ./files/dev
              mkdir -p ./files/tmp
              mkdir -p ./files/run
              mkdir -p ./files/root

              # /nix/var/nix/profiles — needed for nix-env --set
              mkdir -p ./files/nix/var/nix/profiles/per-user/root
              mkdir -p ./files/nix/var/nix/db
              mkdir -p ./files/nix/var/nix/gcroots

              # Create the system profile symlink so /run/current-system
              # can be resolved before postBootCommands runs
              ln -s ${toplevel} ./files/nix/var/nix/profiles/system

              # /etc/os-release — systemd wants this very early
              ln -s ${toplevel}/etc/os-release ./files/etc/os-release

              # ── User-injected files (ch-vm.files) ────────────────
              # Written at build time so they're on disk from first boot.
              ${lib.concatStringsSep "\n" (lib.mapAttrsToList (path: content: ''
                mkdir -p ./files$(dirname ${path})
                cat > ./files${path} <<'__CH_VM_EOF__'
              ${content}
              __CH_VM_EOF__
              '') files)}
            '';
          };
          # ── VM spec ─────────────────────────────────────────────────
          # All the metadata a node/orchestrator needs to launch this VM.
          # The host-module.nix reads allowedDomains to configure nftables.
          vmSpec = {
            inherit hostname;
            kernel = "${nixos.config.system.build.kernel}/${nixos.config.system.boot.loader.kernelFile}";
            initrd = "${nixos.config.system.build.initialRamdisk}/${nixos.config.system.boot.loader.initrdFile}";
            inherit image;
            inherit allowedDomains;
            cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
            inherit entrypoint autoShutdown;
          };

        in { inherit image nixos toplevel vmSpec; };

        # ── mkVmFromDrv: run any derivation in a VM ───────────────
        # The core platform primitive for the gitops workflow.
        #
        # Takes a Nix derivation (any package — a binary, a script,
        # a full application) and produces a VM image that:
        #   1. Boots NixOS with systemd
        #   2. Has the derivation available at its store path
        #   3. Automatically runs the entrypoint on boot
        #   4. Optionally shuts down when the workload exits
        #
        # Arguments:
        #   drv          - the derivation to run (required)
        #   entrypoint   - command to run (default: "${drv}/bin/<pname>")
        #                  Can reference $DRV_PATH for the store path.
        #   autoShutdown - power off when done (default: true)
        #   extraPackages - additional runtime deps (default: none)
        #   files         - extra files to inject (default: {})
        #   sshAuthorizedKeys - SSH keys for debugging (default: [])
        #   hostname     - VM hostname (default: drv.pname or "workload-vm")
        #
        # Usage (in a consuming flake):
        #   ch-vmm.lib.x86_64-linux.mkVmFromDrv {
        #     drv = myApp;  # any derivation with bin/myapp
        #     autoShutdown = true;
        #   }
        mkVmFromDrv = {
          drv,
          entrypoint ? null,
          autoShutdown ? true,
          extraPackages ? (_: []),
          files ? {},
          allowedDomains ? [],
          sshAuthorizedKeys ? [],
          hostname ? (drv.pname or "workload-vm"),
        }: let
          # Default entrypoint: try bin/<pname>, fall back to bin/<name>
          defaultEntrypoint =
            if drv ? meta.mainProgram
            then "${drv}/bin/${drv.meta.mainProgram}"
            else if drv ? pname
            then "${drv}/bin/${drv.pname}"
            else "${drv}/bin/${drv.name}";

          resolvedEntrypoint = if entrypoint != null then entrypoint else defaultEntrypoint;
        in mkVmImage {
          inherit sshAuthorizedKeys hostname autoShutdown files allowedDomains;
          entrypoint = resolvedEntrypoint;
          extraPackages = p: [ drv ] ++ (extraPackages p);
        };

        # ── Default image ──────────────────────────────────────────
        # A ready-to-use minimal image with no extra packages.
        # ~500-700MB. Login: root/root. SSH on port 22.
        default = mkVmImage {};
        defaultImage = default.image;
        defaultKernel = "${default.nixos.config.system.build.kernel}/${default.nixos.config.system.boot.loader.kernelFile}";
        defaultInitrd = "${default.nixos.config.system.build.initialRamdisk}/${default.nixos.config.system.boot.loader.initrdFile}";

        # ── VM launcher script ─────────────────────────────────────
        # One command to:
        #   1. Build cloud-hypervisor (if not built yet)
        #   2. Copy kernel + image to ./vm-artifacts/
        #   3. Launch the VM with serial console
        #
        # The script copies files out of /nix/store because CH needs
        # writable disk images (the VM modifies the rootfs at runtime).
        runVM = pkgs.writeShellScriptBin "run-ch-vm" ''
          #!/usr/bin/env bash
          set -e

          KERNEL="${defaultKernel}"
          INITRD="${defaultInitrd}"
          IMAGE="${defaultImage}"
          WORK_DIR="./vm-artifacts"
          SETUP_NET="${./setup-network.sh}"

          echo "=== Cloud Hypervisor VM (NixOS) ==="
          echo "Kernel:  $KERNEL"
          echo "Initrd:  $INITRD"
          echo "Image:   $IMAGE"
          echo ""

          # ── Build cloud-hypervisor if not present ────────────────
          if [ ! -f ./target/release/cloud-hypervisor ]; then
            echo "Building cloud-hypervisor (this takes a few minutes)..."
            cargo build --release
          fi

          # CH needs CAP_NET_ADMIN to create TAP devices
          sudo setcap cap_net_admin+ep ./target/release/cloud-hypervisor 2>/dev/null || true

          # ── Prepare working directory ────────────────────────────
          mkdir -p "$WORK_DIR"

          # Copy the disk image (it needs to be writable for the VM)
          if [ ! -f "$WORK_DIR/nixos.raw" ]; then
            echo "Copying NixOS disk image (this is a one-time copy)..."
            cp "$IMAGE" "$WORK_DIR/nixos.raw"
            chmod 644 "$WORK_DIR/nixos.raw"
          fi

          # Copy the kernel (read-only is fine, but keeps things tidy)
          if [ ! -f "$WORK_DIR/bzImage" ]; then
            cp "$KERNEL" "$WORK_DIR/bzImage"
            chmod 644 "$WORK_DIR/bzImage"
          fi

          # Copy the initrd (NixOS-built, includes virtio modules)
          if [ ! -f "$WORK_DIR/initrd" ]; then
            cp "$INITRD" "$WORK_DIR/initrd"
            chmod 644 "$WORK_DIR/initrd"
          fi

          # ── Network setup hint ───────────────────────────────────
          if ! ip link show chtap0 &>/dev/null; then
            echo ""
            echo "NOTE: No TAP device found. For networking, run:"
            echo "  sudo bash $SETUP_NET"
            echo ""
            echo "For sandboxed networking (domain allowlist):"
            echo "  sudo bash $SETUP_NET --allow-domain api.openai.com"
            echo ""
            echo "Starting VM without networking..."
            echo ""
            NET_ARGS=""
          else
            NET_ARGS="--net tap=chtap0"
          fi

          echo "Starting VM... (serial console attached)"
          echo "  Login: root / root"
          echo "  Exit:  Ctrl-A x"
          echo ""

          # ── Launch Cloud Hypervisor ──────────────────────────────
          # --kernel:    bzImage for traditional boot
          # --initramfs: NixOS initrd (loads virtio modules, mounts root)
          # --disk:      raw ext4 image, unpartitioned → root=/dev/vda
          # --cmdline:   serial console + root device + read-write mount
          # --serial:    attach serial to the terminal (interactive console)
          # --console:   disable virtio console (we use serial instead)
          # --cpus:      2 vCPUs (adjust as needed)
          # --memory:    512MB (adjust as needed)
          #
          # Any extra arguments you pass to this script are forwarded
          # to cloud-hypervisor (e.g., --memory size=2G).
          ./target/release/cloud-hypervisor \
            --kernel "$WORK_DIR/bzImage" \
            --initramfs "$WORK_DIR/initrd" \
            --disk path="$WORK_DIR/nixos.raw" \
            --cmdline "console=ttyS0 root=/dev/vda rw init=/sbin/init" \
            --serial tty \
            --console off \
            --cpus boot=2 \
            --memory size=512M \
            $NET_ARGS \
            "$@"
        '';

        # ── Sample workload ──────────────────────────────────────────
        # A trivial derivation that demonstrates mkVmFromDrv.
        # It writes system info to a file and prints it to the console.
        # Build the demo image: nix build .#workload-demo-image
        sampleWorkload = pkgs.writeShellScriptBin "sample-workload" ''
          #!/usr/bin/env bash
          echo "========================================="
          echo "  VM Workload Demo"
          echo "  Started at: $(date)"
          echo "  Hostname:   $(hostname)"
          echo "  Kernel:     $(uname -r)"
          echo "  Uptime:     $(uptime -p)"
          echo "========================================="

          # Write a report file (proves workload ran)
          mkdir -p /var/lib/workload
          cat > /var/lib/workload/report.txt <<EOF
          workload=sample-workload
          started=$(date -Iseconds)
          hostname=$(hostname)
          kernel=$(uname -r)
          status=success
          EOF

          echo ""
          echo "Report written to /var/lib/workload/report.txt"
          echo "Workload finished successfully."
        '';

        # Build an image with the sample workload baked in
        workloadDemo = mkVmFromDrv {
          drv = sampleWorkload;
          autoShutdown = true;  # VM powers off when workload is done
          hostname = "demo-vm";
          files = {
            "/etc/motd" = ''
              === Cloud Hypervisor Workload Demo ===
              This VM was built with mkVmFromDrv.
              The workload runs automatically on boot.
            '';
          };
        };

        workloadDemoVM = let
          demoKernel = "${workloadDemo.nixos.config.system.build.kernel}/${workloadDemo.nixos.config.system.boot.loader.kernelFile}";
          demoInitrd = "${workloadDemo.nixos.config.system.build.initialRamdisk}/${workloadDemo.nixos.config.system.boot.loader.initrdFile}";
        in pkgs.writeShellScriptBin "run-workload-demo" ''
          #!/usr/bin/env bash
          set -e

          KERNEL="${demoKernel}"
          INITRD="${demoInitrd}"
          IMAGE="${workloadDemo.image}"
          WORK_DIR="./vm-artifacts"

          echo "=== Workload Demo VM ==="
          echo "This VM runs a sample workload and auto-shuts down."
          echo ""

          if [ ! -f ./target/release/cloud-hypervisor ]; then
            echo "Building cloud-hypervisor..."
            cargo build --release
          fi

          sudo setcap cap_net_admin+ep ./target/release/cloud-hypervisor 2>/dev/null || true

          mkdir -p "$WORK_DIR"

          # Always copy a fresh image (workload VMs are ephemeral)
          echo "Preparing ephemeral disk image..."
          cp "$IMAGE" "$WORK_DIR/workload-demo.raw"
          chmod 644 "$WORK_DIR/workload-demo.raw"

          if [ ! -f "$WORK_DIR/demo-bzImage" ]; then
            cp "$KERNEL" "$WORK_DIR/demo-bzImage"
            chmod 644 "$WORK_DIR/demo-bzImage"
          fi
          if [ ! -f "$WORK_DIR/demo-initrd" ]; then
            cp "$INITRD" "$WORK_DIR/demo-initrd"
            chmod 644 "$WORK_DIR/demo-initrd"
          fi

          echo "Starting workload VM (will auto-shutdown when done)..."
          echo ""

          ./target/release/cloud-hypervisor \
            --kernel "$WORK_DIR/demo-bzImage" \
            --initramfs "$WORK_DIR/demo-initrd" \
            --disk path="$WORK_DIR/workload-demo.raw" \
            --cmdline "console=ttyS0 root=/dev/vda rw init=/sbin/init" \
            --serial tty \
            --console off \
            --cpus boot=2 \
            --memory size=512M \
            "$@"
        '';

      in {
        # ── Dev shell ────────────────────────────────────────────────
        # For building cloud-hypervisor from source.
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            pkg-config
            openssl
            rust-bin-custom
            openssh
            dnsmasq      # For the network setup script
          ];

          shellHook = ''
            echo "=== Cloud Hypervisor VM Builder ==="
            echo ""
            echo "Build commands:"
            echo "  nix build .#kernel   — NixOS kernel (bzImage)"
            echo "  nix build .#image    — Minimal NixOS disk image"
            echo "  nix run   .#vm       — Build + launch VM"
            echo ""
            echo "Workload demo (mkVmFromDrv):"
            echo "  nix build .#workload-demo-image  — Image with baked-in workload"
            echo "  nix run   .#workload-demo        — Launch workload VM (auto-shutdown)"
            echo ""
            echo "Host networking (NixOS module):"
            echo "  Import ch-vmm.nixosModules.host into your host config."
            echo "  See flake-vmm/examples/ for usage."
            echo ""
            echo "Non-NixOS fallback:"
            echo "  sudo bash flake-vmm/setup-network.sh"
            echo "  sudo bash flake-vmm/setup-network.sh --allow-domain api.openai.com"
            echo "  sudo bash flake-vmm/setup-network.sh --cleanup"
            echo ""
            echo "VM credentials: root / root"
            echo ""
          '';
        };

        # ── Library ──────────────────────────────────────────────────
        # The main exports: functions to build custom VM images.
        #
        # Usage in another flake:
        #   ch-vmm.lib.x86_64-linux.mkVmImage {
        #     extraPackages = p: [ p.python3 p.htop ];
        #     sshAuthorizedKeys = [ "ssh-ed25519 AAAA..." ];
        #   }
        #
        #   ch-vmm.lib.x86_64-linux.mkVmFromDrv {
        #     drv = myApp;
        #     autoShutdown = true;
        #   }
        lib = { inherit mkVmImage mkVmFromDrv; };

        # ── Packages ─────────────────────────────────────────────────
        packages = {
          # The NixOS kernel — both boot formats available:
          #   result/bzImage  (compressed, traditional boot)
          #   result/vmlinux  (ELF, PVH direct boot — faster)
          kernel = vmKernel;

          # The default minimal NixOS disk image (~500-700MB).
          # Contains: systemd, SSH, serial console, basic utils.
          # Login: root/root
          image = defaultImage;

          # The VM launcher script
          vm-runner = runVM;

          # ── Workload demo ────────────────────────────────────────
          # Sample derivation and VM image showing mkVmFromDrv in action
          sample-workload = sampleWorkload;
          workload-demo-image = workloadDemo.image;
          workload-demo-runner = workloadDemoVM;

          default = runVM;
        };

        # ── Apps ─────────────────────────────────────────────────────
        apps = {
          # nix run .#vm — build everything and launch the VM
          vm = {
            type = "app";
            program = "${runVM}/bin/run-ch-vm";
          };

          # nix run .#workload-demo — run the sample workload VM
          workload-demo = {
            type = "app";
            program = "${workloadDemoVM}/bin/run-workload-demo";
          };

          default = {
            type = "app";
            program = "${runVM}/bin/run-ch-vm";
          };
        };
      }
    );
}
