{
  description = "NixOS VM images for Cloud Hypervisor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  };

  outputs = {
    self,
    nixpkgs,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};

    kernel = pkgs.linuxPackages_latest.kernel;

    extraPkgs = [
      pkgs.busybox
      pkgs.iproute2
      pkgs.coreutils
      pkgs.python3
      pkgs.git
      pkgs.curl
    ];
    binPaths = builtins.concatStringsSep ":" (map (pkg: "${pkg}/bin") extraPkgs);

    # Simple init script
    initScript = pkgs.writeShellScript "init" ''
      #!/bin/sh
      # export PATH=${binPaths}:/bin
      export PATH=${pkgs.busybox}/bin:${pkgs.bashInteractive}/bin

      # Mount essential filesystems
      mount -t proc proc /proc
      mount -t sysfs sysfs /sys
      mount -t devtmpfs devtmpfs /dev
      mount -t tmpfs tmpfs /tmp
      mount -t tmpfs tmpfs /run

      # Setup hostname
      hostname diskless-vm

      # Setup networking (virtio)
      ip link set lo up
      ip link set eth0 up 2>/dev/null

      echo "Welcome to diskless NixOS VM!"
      echo "$PATH"
      echo "=============================="

      # Drop to shell
      exec setsid cttyhack /bin/sh
      # exec /bin/sh
    '';

    initRoot = pkgs.buildEnv {
      name = "initrd-root";
      paths = extraPkgs;
      pathsToLink = ["/bin"];
    };

    initrd = pkgs.makeInitrd {
      compressor = "zstd";
      contents = [
        {
          object = "${pkgs.busybox}/bin/sh";
          symlink = "/bin/sh";
        }
        {
          object = initScript;
          symlink = "/init";
        }
        {
          object = initRoot;
          symlink = "/";
        }
      ];
    };

    runVM = pkgs.writeShellScriptBin "run-vm" ''
      ${pkgs.cloud-hypervisor}/bin/cloud-hypervisor \
        --kernel ${kernel}/bzImage \
        --initramfs ${initrd}/initrd \
        --cmdline "console=ttyS0 init=/init" \
        --cpus boot=1 \
        --memory size=2G \
        --serial tty \
        --console off
    '';
  in {
    packages.${system} = {
      inherit runVM;
    };

    devShells.${system}.default = pkgs.mkShell {
      buildInputs = [
        pkgs.cloud-hypervisor
      ];
      shellHook = ''
        echo "VMM dev shell — cloud-hypervisor $(cloud-hypervisor --version)"
      '';
    };
  };
}
