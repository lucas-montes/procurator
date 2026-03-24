# mkSandbox — Dockerfile-like API for building Cloud Hypervisor sandbox VMs.
#
# Produces: { image, kernel, initrd, vmSpec, vmSpecJson, launchScript }
#
# Think of this like a Dockerfile:
#   - entrypoint: the command to run (like CMD/ENTRYPOINT)
#   - localFiles: local directory/files to copy into the VM (like COPY/ADD)
#   - packages: nix packages to include (like RUN apt-get install)
#   - cpu/memoryMb: VM resource specs
#   - allowedDomains: network allowlist (empty = full access)
#
# Usage (from a consuming flake):
#   sandbox = procurator.lib.${system}.mkSandbox {
#     entrypoint = "python3 /workspace/main.py";
#     localFiles."/workspace" = ./my-project;
#     packages = p: [ p.python3 p.git ];
#     cpu = 2;
#     memoryMb = 2048;
#     allowedDomains = [ "pypi.org" "api.openai.com" ];
#   };
#
#   # sandbox.image        — ext4 rootfs derivation
#   # sandbox.vmSpecJson    — JSON file with the 8-field VmSpec
#   # sandbox.launchScript  — script to start cloud-hypervisor with this VM

{ pkgs, nixpkgs, system }:

{
  # ── Entrypoint (like CMD in Dockerfile) ──────────────────────────
  # Shell command to run inside the VM after boot.
  entrypoint ? null,

  # ── Auto-shutdown ────────────────────────────────────────────────
  # Power off the VM when the entrypoint exits.
  autoShutdown ? true,

  # ── Packages (like RUN apt-get install) ──────────────────────────
  # Function: pkgs -> [ derivation ]. Packages available in the VM.
  packages ? (_: []),

  # ── Local files (like COPY in Dockerfile) ─────────────────────────
  # Attrset of { guestPath = hostPath; }.
  # hostPath can be a path (directory or file) — it gets copied into
  # the VM image at guestPath.
  # Example: { "/workspace" = ./my-project; "/opt/config.toml" = ./config.toml; }
  localFiles ? {},

  # ── Inline files (string content, like RUN echo "..." > file) ────
  # Attrset of { guestPath = "string content"; }.
  # For small config files where you don't have a local file.
  # Example: { "/etc/myapp/config.toml" = ''listen = "0.0.0.0:8080"''; }
  inlineFiles ? {},

  # ── VM specs ─────────────────────────────────────────────────────
  cpu ? 1,
  memoryMb ? 512,
  hostname ? "sandbox",

  # ── Network ──────────────────────────────────────────────────────
  # Allowlist of domains the VM can reach. Empty = full internet access.
  allowedDomains ? [],

  # ── SSH ──────────────────────────────────────────────────────────
  sshAuthorizedKeys ? [],

  # ── Kernel ───────────────────────────────────────────────────────
  # Custom kernel. Defaults to a minimal config optimized for CH.
  kernel ? null,

  # ── Disk ─────────────────────────────────────────────────────────
  diskSize ? "auto",
  additionalSpace ? "512M",
}:

let
  lib = pkgs.lib;

  # ── Validation ─────────────────────────────────────────────────────
  assertPositiveInt = name: val:
    if !(builtins.isInt val) then
      builtins.throw "mkSandbox: ${name} must be an integer, got ${builtins.typeOf val}"
    else if val <= 0 then
      builtins.throw "mkSandbox: ${name} must be positive, got ${toString val}"
    else true;

  assertFunction = name: val:
    if !(builtins.isFunction val) then
      builtins.throw "mkSandbox: ${name} must be a function (pkgs -> [package]), got ${builtins.typeOf val}"
    else true;

  validated =
    assert assertPositiveInt "cpu" cpu;
    assert assertPositiveInt "memoryMb" memoryMb;
    assert assertFunction "packages" packages;
    assert builtins.isAttrs localFiles || builtins.throw "mkSandbox: localFiles must be an attrset";
    assert builtins.isAttrs inlineFiles || builtins.throw "mkSandbox: inlineFiles must be an attrset";
    assert builtins.isList allowedDomains || builtins.throw "mkSandbox: allowedDomains must be a list";
    true;

  # ── Resolve kernel ─────────────────────────────────────────────────
  # Use the custom minimal kernel config, or fall back to a stock kernel.
  sandboxKernel =
    if kernel != null then kernel
    else (import ./kernel.nix { inherit pkgs; });

  # ── Copy local paths into a single store derivation ────────────────
  # Each entry in localFiles becomes a directory/file in the image.
  # We use pkgs.runCommand to copy everything into a structured tree.
  localFilesStore = pkgs.runCommand "sandbox-local-files" {} (
    ''
      mkdir -p $out
    '' + lib.concatStringsSep "\n" (lib.mapAttrsToList (guestPath: hostPath:
      let
        src = builtins.path { path = hostPath; name = builtins.baseNameOf (toString hostPath); };
      in ''
        mkdir -p $out$(dirname ${guestPath})
        cp -rL ${src} $out${guestPath}
        chmod -R u+rw $out${guestPath}
      ''
    ) localFiles)
  );

  hasLocalFiles = localFiles != {};

  # ── Build the NixOS system ─────────────────────────────────────────
  nixos = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      # Base sandbox VM module (minimal NixOS for CH)
      ./sandbox-module.nix

      # Use the sandbox kernel
      { boot.kernelPackages = pkgs.linuxPackagesFor sandboxKernel; }

      # Apply user configuration
      ({ pkgs, ... }: {
        ch-sandbox = {
          inherit hostname entrypoint autoShutdown allowedDomains sshAuthorizedKeys;
          extraPackages = packages pkgs;
          inlineFiles = inlineFiles;
          localFilesPath = if hasLocalFiles then localFilesStore else null;
        };
      })

      # ── First-boot Nix store registration ────────────────────────
      ({ config, pkgs, ... }: {
        boot.postBootCommands = ''
          if [ -f /nix-path-registration ]; then
            set -euo pipefail
            ${pkgs.nix}/bin/nix-store --load-db < /nix-path-registration
            touch /etc/NIXOS
            ${pkgs.nix}/bin/nix-env -p /nix/var/nix/profiles/system --set /run/current-system
            rm -f /nix-path-registration
          fi
        '';
      })
    ];
  };

  toplevel = nixos.config.system.build.toplevel;

  # ── Build the ext4 disk image ──────────────────────────────────────
  makeExt4FsFn = import "${nixpkgs}/nixos/lib/make-ext4-fs.nix";
  makeExt4FsArgs = builtins.functionArgs makeExt4FsFn;

  image = pkgs.callPackage makeExt4FsFn (
    {
      compressImage = false;
      volumeLabel = "nixos";
      storePaths = [toplevel];
      populateImageCommands = ''
        # Minimal filesystem skeleton
        mkdir -p ./files/sbin
        ln -s ${toplevel}/init ./files/sbin/init

        mkdir -p ./files/etc
        touch ./files/etc/NIXOS

        mkdir -p ./files/var/log
        mkdir -p ./files/var/lib
        mkdir -p ./files/proc
        mkdir -p ./files/sys
        mkdir -p ./files/dev
        mkdir -p ./files/tmp
        mkdir -p ./files/run
        mkdir -p ./files/root

        mkdir -p ./files/nix/var/nix/profiles/per-user/root
        mkdir -p ./files/nix/var/nix/db
        mkdir -p ./files/nix/var/nix/gcroots

        ln -s ${toplevel} ./files/nix/var/nix/profiles/system
        ln -s ${toplevel}/etc/os-release ./files/etc/os-release

        # Inline files (string content)
        ${lib.concatStringsSep "\n" (lib.mapAttrsToList (path: content: ''
          mkdir -p ./files$(dirname ${path})
          cat > ./files${path} <<'__SANDBOX_EOF__'
        ${content}
        __SANDBOX_EOF__
        '') inlineFiles)}

        # Local files (copied from host paths)
        ${lib.optionalString hasLocalFiles ''
          echo "Copying local files into image..."
          cp -rL ${localFilesStore}/. ./files/
          chmod -R u+rw ./files/
        ''}
      '';
    }
    // lib.optionalAttrs (makeExt4FsArgs ? additionalSpace) {
      inherit additionalSpace;
    }
    // lib.optionalAttrs (makeExt4FsArgs ? diskSize) {
      inherit diskSize;
    }
  );

  # ── VM spec (matches 8-field capnp VmSpec) ─────────────────────────
  vmSpec = {
    toplevel = toString toplevel;
    kernelPath = "${nixos.config.system.build.kernel}/${nixos.config.system.boot.loader.kernelFile}";
    initrdPath = "${nixos.config.system.build.initialRamdisk}/${nixos.config.system.boot.loader.initrdFile}";
    diskImagePath = toString image;
    cmdline = "console=ttyS0 root=/dev/vda rw init=/sbin/init";
    inherit cpu memoryMb;
    networkAllowedDomains = allowedDomains;
  };

  vmSpecJson = pkgs.writeText "sandbox-vm-spec.json" (builtins.toJSON vmSpec);

  # ── Launch script ──────────────────────────────────────────────────
  launchScript = import ./launch-script.nix {
    inherit pkgs vmSpec image;
    kernelPath = vmSpec.kernelPath;
    initrdPath = vmSpec.initrdPath;
    diskImagePath = toString image;
    cmdline = vmSpec.cmdline;
    inherit cpu memoryMb hostname;
  };

in
  assert validated;
  {
    inherit image nixos toplevel vmSpec vmSpecJson launchScript;

    # Convenience: just the paths for manual cloud-hypervisor invocation
    paths = {
      kernel = vmSpec.kernelPath;
      initrd = vmSpec.initrdPath;
      disk = toString image;
    };
  }
