# mkSandbox — Dockerfile-like API for Cloud Hypervisor Sandbox VMs

A Nix function that builds a minimal NixOS VM image for Cloud Hypervisor, with a Dockerfile-like interface for defining what goes inside.

## Quick Start

```nix
# flake.nix
{
  inputs.procurator.url = "github:lucas-montes/procurator";

  outputs = { procurator, ... }: let
    sandbox = procurator.lib.x86_64-linux.mkSandbox {
      entrypoint = "python3 /workspace/app.py";
      localFiles."/workspace" = ./my-project;
      packages = p: [ p.python3 p.git ];
      cpu = 2;
      memoryMb = 2048;
      allowedDomains = [ "pypi.org" "api.openai.com" ];
    };
  in {
    packages.x86_64-linux.default = sandbox.launchScript;
  };
}
```

Then:

```bash
nix run .#default
```

## API Reference

### `mkSandbox { ... }`

Returns: `{ image, nixos, toplevel, vmSpec, vmSpecJson, launchScript, paths }`

#### Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `entrypoint` | `string \| null` | `null` | Shell command to run after boot (like `CMD`) |
| `autoShutdown` | `bool` | `true` | Power off VM when entrypoint exits |
| `packages` | `pkgs -> [drv]` | `_ : []` | Packages to install (like `apt-get install`) |
| `localFiles` | `{ path = ./src; }` | `{}` | Local paths to copy into the VM (like `COPY`) |
| `inlineFiles` | `{ path = "content"; }` | `{}` | Files as string content (like `RUN echo > file`) |
| `cpu` | `int` | `1` | Number of vCPUs |
| `memoryMb` | `int` | `512` | RAM in megabytes |
| `hostname` | `string` | `"sandbox"` | VM hostname |
| `allowedDomains` | `[string]` | `[]` | Domain allowlist (empty = full access) |
| `sshAuthorizedKeys` | `[string]` | `[]` | SSH public keys for root |
| `kernel` | `derivation \| null` | custom minimal | Custom kernel (defaults to minimal CH kernel) |
| `diskSize` | `string` | `"auto"` | Disk image size |
| `additionalSpace` | `string` | `"512M"` | Extra disk space beyond store contents |

#### Return Value

| Field | Description |
|-------|-------------|
| `image` | ext4 rootfs derivation (read-only, Nix store) |
| `nixos` | Full NixOS system configuration |
| `toplevel` | NixOS system.build.toplevel |
| `vmSpec` | Attrset matching the 8-field capnp VmSpec |
| `vmSpecJson` | Derivation producing `sandbox-vm-spec.json` |
| `launchScript` | Shell script that launches cloud-hypervisor |
| `paths.kernel` | Path to the kernel image |
| `paths.initrd` | Path to the initrd |
| `paths.disk` | Path to the ext4 rootfs |

## Dockerfile Analogy

| Dockerfile | mkSandbox |
|------------|-----------|
| `FROM ubuntu` | NixOS minimal (automatic) |
| `RUN apt-get install python3` | `packages = p: [ p.python3 ];` |
| `COPY ./src /app` | `localFiles."/app" = ./src;` |
| `RUN echo "config" > /etc/app.conf` | `inlineFiles."/etc/app.conf" = "config";` |
| `CMD ["python3", "app.py"]` | `entrypoint = "python3 app.py";` |
| `EXPOSE 8080` | (VM-level, handled by network) |

## Network Security

### Domain Allowlist

When `allowedDomains` is non-empty, **guest-side nftables** rules are installed at boot:

1. Resolve each domain to IPs via DNS
2. Allow traffic only to those IPs + DNS + DHCP + loopback
3. Drop everything else

This makes the VM a secure sandbox — code inside cannot phone home to arbitrary servers.

```nix
sandbox = mkSandbox {
  entrypoint = "python3 /workspace/agent.py";
  allowedDomains = [
    "api.openai.com"     # LLM API
    "pypi.org"           # Package installs
    "files.pythonhosted.org"
  ];
  # Everything else is blocked
};
```

### Defense in Depth

The domain allowlist operates at two levels:

1. **Guest-side** (this library): nftables inside the VM — always active
2. **Host-side** (optional, via `ch-host` NixOS module): nftables on the host bridge — production hardening

## Launch Script

The `launchScript` handles:

1. Creates a **writable copy** of the rootfs (Nix store images are read-only)
2. Sets up a TAP device on the host bridge (if the bridge exists)
3. Starts `cloud-hypervisor` with the correct kernel/initrd/disk/cmdline
4. Cleans up on exit (TAP device, writable disk copy)

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SANDBOX_VM_DIR` | `/tmp/sandbox-vms` | Base directory for VM runtime files |
| `SANDBOX_BRIDGE` | `chbr0` | Bridge name (must exist, or VM runs without network) |
| `SANDBOX_SERIAL` | `tty` | `tty` for interactive serial, `file` for log file |

### Prerequisites

The launch script requires:

- `cloud-hypervisor` installed (or in the Nix store)
- A bridge device (`chbr0`) if you want networking (see Host Setup below)
- `CAP_NET_ADMIN` capability for TAP creation (or run as root)

### Host Network Setup

For VMs to have network access, the host needs a bridge with NAT. On NixOS, use the `ch-host` module:

```nix
# In your host configuration.nix
imports = [ procurator.nixosModules.${system}.host ];

ch-host = {
  enable = true;
  bridge.address = "192.168.249.1";
};
```

Or manually:

```bash
# Create bridge
sudo ip link add chbr0 type bridge
sudo ip addr add 192.168.249.1/24 dev chbr0
sudo ip link set chbr0 up

# Enable NAT
sudo sysctl net.ipv4.ip_forward=1
sudo iptables -t nat -A POSTROUTING -s 192.168.249.0/24 -j MASQUERADE

# Start dnsmasq for DHCP
sudo dnsmasq --interface=chbr0 --bind-interfaces \
  --dhcp-range=192.168.249.10,192.168.249.50,12h \
  --dhcp-option=3,192.168.249.1 \
  --dhcp-option=6,192.168.249.1
```

## Integration with Procurator Worker

The `vmSpecJson` output is compatible with the procurator worker's `createVm` RPC. To deploy via the worker instead of the launch script:

```bash
# Build the VM spec
nix build .#vmSpec

# Send to worker
pcr-test --addr 127.0.0.1:6000 create-vm --spec-file ./result
```

The worker handles TAP creation, bridge attachment, and lifecycle management automatically.

## Custom Kernel

The default kernel is a minimal build (~10-15MB) with only:
- Virtio (PCI, block, net, console, SCSI)
- ext4
- Serial console
- TCP/IP + netfilter (for nftables)
- systemd requirements

To use a stock kernel instead:

```nix
sandbox = mkSandbox {
  kernel = pkgs.linux_6_6;  # Stock kernel (~50MB)
  # ... rest of config
};
```

## Example

See `nix/examples/sandbox/` for a complete working example that:
1. Copies a Python script into the VM
2. Runs it as the entrypoint
3. Tests domain allowlisting
4. Writes structured results for host-side collection
