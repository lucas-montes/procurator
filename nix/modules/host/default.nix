# host-module.nix — NixOS module for the HOST that runs Cloud Hypervisor VMs
#
# This replaces setup-network.sh with pure NixOS configuration.
# Import it into your host's NixOS config and declare your VMs:
#
#   # In your host's configuration.nix or flake module:
#   imports = [ ch-vmm.nixosModules.host ];
#
#   ch-host.vms = {
#     my-sandbox = {
#       allowedDomains = [ "api.openai.com" "github.com" ];
#       # tapName auto-generated as "chvm-my-sandbox" (or override)
#     };
#     ci-runner = {
#       # empty allowedDomains = full internet access
#     };
#   };
#
# What it creates (per VM):
#   - A TAP device for CH's --net flag
#   - Attached to a shared bridge (chbr0)
#   - dnsmasq on the bridge (DHCP + DNS for guests)
#   - NAT/masquerade for outbound traffic
#   - nftables rules for domain allowlisting (when configured)
#
# The TAP device name is printed by `systemctl status ch-vm-tap-<name>`
# and is what you pass to cloud-hypervisor: --net tap=chvm-<name>

{ config, pkgs, lib, ... }:

let
  cfg = config.ch-host;

  # Sanitize VM name to a valid interface name (max 15 chars for Linux)
  mkTapName = name: let
    # "chvm-" prefix (5 chars) + truncated name (10 chars max)
    sanitized = lib.replaceStrings ["-" "_" "."] ["" "" ""] name;
  in "chvm-${builtins.substring 0 10 sanitized}";

  # Resolve domains to IPs at activation time via a script
  # (nftables sets can't do DNS natively, so we resolve + insert)
  mkDomainResolveScript = vmName: domains: pkgs.writeShellScript "resolve-domains-${vmName}" ''
    set -euo pipefail
    export PATH="${lib.makeBinPath (with pkgs; [ nftables glibc.bin iproute2 coreutils ])}"

    TABLE="ch_vm_${lib.replaceStrings ["-"] ["_"] vmName}"

    # Flush and recreate the nftables table for this VM
    nft delete table inet "$TABLE" 2>/dev/null || true
    nft add table inet "$TABLE"
    nft add chain inet "$TABLE" forward '{ type filter hook forward priority 0; policy accept; }'

    # Only filter traffic FROM the bridge that's NOT going to the bridge
    # (i.e., VM→internet traffic). Bridge-internal traffic is always allowed.
    nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" oifname "${cfg.bridge.name}" accept

    # Always allow DNS (so resolution works inside the VM)
    nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" udp dport 53 accept
    nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" tcp dport 53 accept

    # Allow established/related (return traffic)
    nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" ct state established,related accept

    # Resolve each domain and allow its IPs
    ${lib.concatMapStringsSep "\n" (domain: ''
      echo "Resolving ${domain}..."
      for ip in $(getent ahostsv4 "${domain}" 2>/dev/null | awk '{print $1}' | sort -u); do
        echo "  Allowing $ip (${domain})"
        nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" ip daddr "$ip" accept
      done
    '') domains}

    # Drop everything else from the VM going outbound
    nft add rule inet "$TABLE" forward iifname "${cfg.bridge.name}" oifname != "${cfg.bridge.name}" drop

    echo "Domain allowlist active for ${vmName}: ${toString (builtins.length domains)} domains"
  '';

in {
  options.ch-host = {
    enable = lib.mkEnableOption "Cloud Hypervisor VM host networking";

    bridge = {
      name = lib.mkOption {
        type = lib.types.str;
        default = "chbr0";
        description = "Name of the bridge device connecting all VM TAPs.";
      };

      address = lib.mkOption {
        type = lib.types.str;
        default = "192.168.249.1";
        description = "IP address of the bridge (gateway for VMs).";
      };

      subnet = lib.mkOption {
        type = lib.types.str;
        default = "192.168.249.0/24";
        description = "Subnet for the VM bridge network.";
      };

      prefixLength = lib.mkOption {
        type = lib.types.int;
        default = 24;
        description = "Prefix length for the bridge address.";
      };
    };

    dhcp = {
      rangeStart = lib.mkOption {
        type = lib.types.str;
        default = "192.168.249.10";
        description = "Start of DHCP range for VMs.";
      };

      rangeEnd = lib.mkOption {
        type = lib.types.str;
        default = "192.168.249.50";
        description = "End of DHCP range for VMs.";
      };
    };

    vms = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
        options = {
          tapName = lib.mkOption {
            type = lib.types.str;
            default = mkTapName name;
            description = ''
              TAP device name for this VM. Defaults to "chvm-<name>".
              This is what you pass to cloud-hypervisor: --net tap=<tapName>
            '';
          };

          allowedDomains = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = ''
              Domains this VM is allowed to reach. Empty = full access.
              When set, nftables rules allow only DNS + these domains.
            '';
            example = [ "api.openai.com" "github.com" ];
          };
        };
      }));
      default = {};
      description = ''
        Attrset of VM names to their network config.
        Each VM gets a dedicated TAP device attached to the shared bridge.
      '';
      example = lib.literalExpression ''
        {
          sandbox = {
            allowedDomains = [ "api.openai.com" ];
          };
          ci-runner = {}; # full internet
        }
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # ── IP forwarding ──────────────────────────────────────────────
    boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

    # ── Bridge + TAP netdevs ──────────────────────────────────────
    # systemd-networkd creates and manages the bridge and all TAPs.
    systemd.network.enable = true;

    systemd.network.netdevs = {
      "10-${cfg.bridge.name}" = {
        netdevConfig = {
          Name = cfg.bridge.name;
          Kind = "bridge";
        };
      };
    } // lib.mapAttrs' (_: vmCfg:
      lib.nameValuePair "20-${vmCfg.tapName}" {
        netdevConfig = {
          Name = vmCfg.tapName;
          Kind = "tap";
        };
        tapConfig = {
          User = "root";
          Group = "root";
        };
      }
    ) cfg.vms;

    systemd.network.networks = {
      "10-${cfg.bridge.name}" = {
        matchConfig.Name = cfg.bridge.name;
        networkConfig.DHCPServer = false;
        address = [ "${cfg.bridge.address}/${toString cfg.bridge.prefixLength}" ];
        linkConfig.RequiredForOnline = "no";
      };
    } // lib.mapAttrs' (_: vmCfg:
      lib.nameValuePair "20-${vmCfg.tapName}" {
        matchConfig.Name = vmCfg.tapName;
        networkConfig.Bridge = cfg.bridge.name;
        linkConfig.RequiredForOnline = "no";
      }
    ) cfg.vms;

    # ── NAT / Masquerade ───────────────────────────────────────────
    # Outbound traffic from VMs gets NATed through the host.
    networking.nat = {
      enable = true;
      internalInterfaces = [ cfg.bridge.name ];
      # externalInterface is usually auto-detected; user can override
    };

    # ── dnsmasq (DHCP + DNS for VMs) ──────────────────────────────
    services.dnsmasq = {
      enable = true;
      settings = {
        interface = cfg.bridge.name;
        bind-interfaces = true;
        dhcp-range = "${cfg.dhcp.rangeStart},${cfg.dhcp.rangeEnd},12h";
        # Gateway = bridge IP, DNS = bridge IP (dnsmasq forwards)
        dhcp-option = [
          "3,${cfg.bridge.address}"
          "6,${cfg.bridge.address}"
        ];
        # Don't interfere with the host's own DNS
        except-interface = "lo";
        # Log for debugging (journalctl -u dnsmasq)
        log-queries = true;
        log-dhcp = true;
      };
    };

    # ── nftables domain allowlisting ──────────────────────────────
    # For each VM with allowedDomains, create a systemd service that
    # resolves domains → IPs and installs nftables rules.
    #
    # Runs at boot and can be restarted to refresh DNS:
    #   systemctl restart ch-vm-firewall-<name>
    networking.nftables.enable = true;

    systemd.services = lib.mkMerge (lib.mapAttrsToList (vmName: vmCfg:
      lib.optionalAttrs (vmCfg.allowedDomains != []) {
        "ch-vm-firewall-${vmName}" = {
          description = "Domain allowlist firewall for VM: ${vmName}";
          wantedBy = [ "multi-user.target" ];
          after = [ "network-online.target" "nftables.service" ];
          wants = [ "network-online.target" ];
          path = with pkgs; [ nftables glibc.bin iproute2 coreutils gawk ];

          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = mkDomainResolveScript vmName vmCfg.allowedDomains;
            # On stop, clean up the nftables table
            ExecStop = pkgs.writeShellScript "cleanup-firewall-${vmName}" ''
              nft delete table inet "ch_vm_${lib.replaceStrings ["-"] ["_"] vmName}" 2>/dev/null || true
            '';
          };
        };
      }
    ) cfg.vms);
  };
}
