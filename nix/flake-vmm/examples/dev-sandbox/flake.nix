# Example: Interactive dev sandbox VM with domain allowlisting
#
# This shows how to use mkVmImage for a long-running interactive VM
# that developers SSH into, with curated packages and network sandboxing.
#
# It also demonstrates the host NixOS module for declarative networking.
#
# Usage:
#   cd flake-vmm/examples/dev-sandbox
#   nix build .#vm-image    # build the VM image
#   nix build .#host-config-example  # see what your host config looks like
#
# Architecture:
#   ┌─────────────────────────────────────────────┐
#   │  NixOS HOST                                 │
#   │  ├── ch-host.enable = true                  │
#   │  ├── bridge: chbr0 (192.168.249.1/24)       │
#   │  ├── TAP: chvm-devsandbo (auto-generated)   │
#   │  ├── dnsmasq: DHCP on bridge                │
#   │  ├── NAT: masquerade outbound                │
#   │  └── nftables: only allow github + pypi      │
#   │                                              │
#   │  ┌────────────────────────────────────────┐  │
#   │  │  NixOS GUEST (VM)                      │  │
#   │  │  ├── python3, git, jq, htop, vim       │  │
#   │  │  ├── SSH on port 22 (key auth)         │  │
#   │  │  ├── serial console (root autologin)   │  │
#   │  │  └── no entrypoint (interactive use)   │  │
#   │  └────────────────────────────────────────┘  │
#   └─────────────────────────────────────────────┘

{
  description = "Interactive dev sandbox VM with domain allowlisting";

  inputs = {
    ch-vmm.url = "path:../..";  # points to flake-vmm/
    nixpkgs.follows = "ch-vmm/nixpkgs";
  };

  outputs = { ch-vmm, nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      chLib = ch-vmm.lib.${system};

      # ── VM image ───────────────────────────────────────────────
      # mkVmImage: a general-purpose dev sandbox.
      # No entrypoint — this VM stays running for SSH access.
      vm = chLib.mkVmImage {
        hostname = "dev-sandbox";

        # Packages available inside the VM
        extraPackages = p: [
          p.python3
          p.git
          p.jq
          p.htop
          p.vim
          p.tmux
          p.ripgrep
          p.fd
        ];

        # SSH keys for the dev team (replace with your own)
        sshAuthorizedKeys = [
          # "ssh-ed25519 AAAA... user@machine"
        ];

        # Domain allowlist — VM can ONLY reach these.
        # The host's nftables rules enforce this.
        allowedDomains = [
          "github.com"
          "api.github.com"
          "pypi.org"
          "files.pythonhosted.org"
        ];

        # No entrypoint — interactive use via SSH
        # entrypoint = null; (default)
        # autoShutdown = false; (default)

        # Inject dev environment config
        files = {
          "/root/.bashrc" = ''
            export PS1='\[\e[1;32m\][dev-sandbox]\[\e[0m\] \w\$ '
            alias ll='ls -la'
            alias gs='git status'
            echo ""
            echo "=== Dev Sandbox ==="
            echo "Allowed domains: github.com, pypi.org"
            echo "Packages: python3, git, jq, htop, vim, tmux, ripgrep, fd"
            echo ""
          '';
          "/root/.vimrc" = ''
            set number
            set expandtab
            set shiftwidth=4
            syntax on
          '';
        };
      };

      # ── Runner (for dev/testing without full host setup) ───────
      vmRunner = pkgs.writeShellScriptBin "run-dev-sandbox" ''
        #!/usr/bin/env bash
        set -e
        WORK_DIR="./vm-artifacts"
        mkdir -p "$WORK_DIR"

        echo "=== Dev Sandbox VM ==="
        echo "Interactive NixOS VM with SSH access."
        echo ""

        cp "${vm.image}" "$WORK_DIR/dev-sandbox.raw"
        chmod 644 "$WORK_DIR/dev-sandbox.raw"

        KERNEL="${vm.vmSpec.kernel}"
        INITRD="${vm.vmSpec.initrd}"

        [ -f "$WORK_DIR/dev-bzImage" ] || cp "$KERNEL" "$WORK_DIR/dev-bzImage"
        [ -f "$WORK_DIR/dev-initrd" ]  || cp "$INITRD"  "$WORK_DIR/dev-initrd"
        chmod 644 "$WORK_DIR/dev-bzImage" "$WORK_DIR/dev-initrd"

        CH="''${CH_BIN:-./target/release/cloud-hypervisor}"
        if [ ! -f "$CH" ]; then
          echo "cloud-hypervisor not found at $CH"
          echo "Set CH_BIN= or build with: cargo build --release"
          exit 1
        fi

        TAP_NAME="''${TAP:-chvm-devsandbo}"
        if ip link show "$TAP_NAME" &>/dev/null; then
          NET_ARGS="--net tap=$TAP_NAME"
          echo "Networking: TAP=$TAP_NAME (domain filtering active on host)"
        else
          echo "No TAP device '$TAP_NAME'. Running without networking."
          echo ""
          echo "For NixOS hosts, add to your configuration.nix:"
          echo "  imports = [ ch-vmm.nixosModules.host ];"
          echo "  ch-host.enable = true;"
          echo "  ch-host.vms.dev-sandbox.allowedDomains = [ \"github.com\" \"pypi.org\" ];"
          NET_ARGS=""
        fi

        echo ""
        echo "Login: root / root (or SSH with your key)"
        echo "Exit:  Ctrl-A x"
        echo ""

        "$CH" \
          --kernel "$WORK_DIR/dev-bzImage" \
          --initramfs "$WORK_DIR/dev-initrd" \
          --disk path="$WORK_DIR/dev-sandbox.raw" \
          --cmdline "${vm.vmSpec.cmdline}" \
          --serial tty \
          --console off \
          --cpus boot=4 \
          --memory size=2G \
          $NET_ARGS \
          "$@"
      '';

      # ── Example host NixOS config ─────────────────────────────
      # This is what the host machine's configuration.nix would look like.
      # It's exposed as a package so you can inspect it.
      hostConfigExample = pkgs.writeText "host-configuration-example.nix" ''
        # Add this to your NixOS host's configuration.nix (or a module):
        #
        # imports = [ ch-vmm.nixosModules.host ];
        #
        # Then:
        { config, ... }: {
          ch-host = {
            enable = true;

            # Bridge configuration (defaults are usually fine)
            # bridge.name = "chbr0";
            # bridge.address = "192.168.249.1";
            # bridge.subnet = "192.168.249.0/24";

            # DHCP range for VMs
            # dhcp.rangeStart = "192.168.249.10";
            # dhcp.rangeEnd = "192.168.249.50";

            # Declare your VMs — each gets a TAP device + firewall rules
            vms = {
              dev-sandbox = {
                # tapName is auto-generated: "chvm-devsandbo"
                # Use --net tap=chvm-devsandbo when launching CH
                allowedDomains = [
                  "github.com"
                  "api.github.com"
                  "pypi.org"
                  "files.pythonhosted.org"
                ];
              };

              # Another VM with full internet access
              # ci-runner = {};

              # Another VM with different restrictions
              # llm-sandbox = {
              #   allowedDomains = [ "api.openai.com" "api.anthropic.com" ];
              # };
            };
          };
        }
      '';

    in {
      packages.${system} = {
        vm-image = vm.image;
        run-vm = vmRunner;
        host-config-example = hostConfigExample;

        vm-spec = pkgs.writeText "vm-spec.json" (builtins.toJSON {
          inherit (vm.vmSpec) hostname kernel initrd cmdline
                              allowedDomains entrypoint autoShutdown;
          image = toString vm.image;
        });

        default = vm.image;
      };

      apps.${system} = {
        run-vm = { type = "app"; program = "${vmRunner}/bin/run-dev-sandbox"; };
        default = { type = "app"; program = "${vmRunner}/bin/run-dev-sandbox"; };
      };
    };
}
