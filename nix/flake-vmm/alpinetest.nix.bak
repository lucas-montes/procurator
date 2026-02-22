{
  description = "Cloud Hypervisor Development with Custom Kernel and Ubuntu Rootfs";

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
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust-bin-custom = pkgs.rust-bin.stable."1.88.0".default.override {
          extensions = ["rust-src" "rust-analyzer"];
        };

        # Use the provided Firecracker microvm kernel config file to avoid
        # malformed generated config. This loads the config directly.
        customKernel = pkgs.linux_6_6.override {
          configfile = ./microvm-kernel-ci-x86_64-6.1.config;
          structuredExtraConfig = with pkgs.lib.kernel; {
            # Force essential virtio and filesystem drivers built-in
            VIRTIO = yes;
            VIRTIO_PCI = yes;
            VIRTIO_BLK = yes;
            VIRTIO_NET = yes;
            VIRTIO_CONSOLE = yes;

            # Filesystem and serial console
            EXT4_FS = yes;
            SERIAL_8250 = yes;
            SERIAL_8250_CONSOLE = yes;
          };
        };

        # Kernel packages for the custom kernel
        customKernelPackages = pkgs.linuxPackagesFor customKernel;

        # Fetch Alpine minirootfs deterministically
        alpineTarball = pkgs.fetchurl {
          url = "https://dl-cdn.alpinelinux.org/alpine/v3.19/releases/x86_64/alpine-minirootfs-3.19.6-x86_64.tar.gz";
          sha256 = "sha256-lR4Hb1KInn4vbBmJG4u92egALTblh4exarQvt+R2RKg=";
        };

        # Create a minimal Alpine-based real ext4 raw disk image (real block device)
        alpineImage = pkgs.runCommand "alpine-rootfs" {
          nativeBuildInputs = with pkgs; [ gnutar ];
          buildInputs = [ pkgs.e2fsprogs ];
        } ''
          set -e
          mkdir -p rootfs

          # Unpack the fetched Alpine minirootfs
          tar -xzf ${alpineTarball} -C rootfs

          # Add a simple root user and copy test.py into /root if present
          mkdir -p rootfs/root
          if [ -f "${./test.py}" ]; then
            cp ${./test.py} rootfs/root/test.py || true
            chmod +x rootfs/root/test.py || true
          fi

          # Minimal passwd (static) so you can login if you install ssh later
          cat > rootfs/etc/passwd <<'EOF'
          root:x:0:0:root:/root:/bin/sh
          EOF

          # Create a 512M raw ext4 image containing the rootfs
          mkdir -p $out
          ${pkgs.e2fsprogs}/bin/mke2fs -t ext4 -d rootfs -r 1 -N 0 -m 1 -L "alpine-root" $out/alpine-rootfs.raw 512M
        '';

        # VM runner script using the custom kernel and the real raw disk image
        runVM = pkgs.writeShellScriptBin "run-ch-vm" ''
          #!/usr/bin/env bash
          set -e

          echo "=== Cloud Hypervisor VM (custom kernel + alpine raw disk) ==="
          echo "Kernel: ${customKernel}/bzImage"
          echo "Disk: ${alpineImage}/alpine-rootfs.raw"
          echo ""

          # Build cloud-hypervisor if not present
          if [ ! -f ./target/release/cloud-hypervisor ]; then
            echo "Building cloud-hypervisor..."
            cargo build --release
          fi

          sudo setcap cap_net_admin+ep ./target/release/cloud-hypervisor || true

          WORK_DIR="./vm-artifacts"
          mkdir -p "$WORK_DIR"

          if [ ! -f "$WORK_DIR/alpine.raw" ]; then
            echo "Copying raw disk image..."
            cp ${alpineImage}/alpine-rootfs.raw "$WORK_DIR/alpine.raw"
            chmod 644 "$WORK_DIR/alpine.raw"
          fi

          if [ ! -f "$WORK_DIR/bzImage" ]; then
            echo "Copying kernel bzImage..."
            cp ${customKernel}/bzImage "$WORK_DIR/bzImage"
            chmod 644 "$WORK_DIR/bzImage"
          fi

          echo "Starting VM..."
          echo "Login: root (use console or install ssh in the image)"
          echo ""

          ./target/release/cloud-hypervisor \
            --kernel $WORK_DIR/bzImage \
            --disk path="$WORK_DIR/alpine.raw" \
            --cmdline "console=ttyS0 root=/dev/vda rw" \
            --serial tty \
            --console off \
            --cpus boot=2 \
            --memory size=512M \
            --net "tap=,mac=,ip=,mask=" \
            "$@"
        '';

        # Script to run test inside VM via SSH (if networking is set up)
        testRunner = pkgs.writeShellScriptBin "test-vm" ''
          #!/usr/bin/env bash
          echo "This script would SSH into the VM and run the test"
          echo "First start the VM with: nix run .#vm"
          echo "Then find the VM's IP and run: ssh cloud@<VM_IP>"
          echo "Password: cloud"
          echo "Then run: python3 test.py"
        '';

   in {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            pkg-config
            openssl
            rust-bin-custom
            openssh
          ];

          shellHook = ''
            echo "=== Cloud Hypervisor Development ==="
            echo "Available commands:"
            echo "  nix build .#kernel           - Build custom Linux kernel (with virtio/ext4)"
            echo "  nix build .#alpine-image     - Build minimal Alpine raw disk image"
            echo "  nix run .#vm                 - Run VM with custom kernel + Alpine raw disk"
            echo ""
            echo "VM will start with:"
            echo "  - Custom minimal kernel (built from nixpkgs linux_6_6 with virtio built-in)"
            echo "  - Alpine minirootfs raw ext4 image (real block device)"
            echo "  - Login via the serial console as root (no cloud user by default)"
            echo ""
          '';
        };

        packages = {
          kernel = customKernel;
          "alpine-image" = alpineImage;
          vm-runner = runVM;
          test-runner = testRunner;
          default = runVM;
        };

        apps = {
          vm = {
            type = "app";
            program = "${runVM}/bin/run-ch-vm";
          };
          default = {
            type = "app";
            program = "${runVM}/bin/run-ch-vm";
          };
        };
      }
    );
}
