# Procurator NixOS Service Modules

## Usage

Add procurator to your NixOS configuration:

### Option 1: Using Cluster Blueprint (Recommended)

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    procurator.url = "git+file:///home/lucas/Projects/procurator?dir=nix";
  };

  outputs = { nixpkgs, procurator, ... }: {
    nixosConfigurations.my-worker = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        procurator.nixosModules.cluster
        procurator.nixosModules.procurator-worker

        {
          # Define cluster topology
          cluster.vms = {
            control-plane-1 = {
              role = "control-plane";
              deployment.addr = "192.168.1.10:8080";
              # ... other config
            };
            worker-1 = {
              role = "worker";
              deployment.addr = "192.168.1.11:8080";
              # ... other config
            };
          };

          # Worker references master by name
          services.procurator.worker = {
            enable = true;
            addr = "0.0.0.0:8080";
            master = "control-plane-1";  # Automatically uses its addr
          };
        }
      ];
    };

    nixosConfigurations.my-control-plane = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        procurator.nixosModules.cluster
        procurator.nixosModules.procurator-control-plane

        {
          cluster.vms = {
            control-plane-1 = {
              role = "control-plane";
              deployment.addr = "192.168.1.10:8080";
            };
            worker-1 = {
              role = "worker";
              deployment.addr = "192.168.1.11:8080";
            };
            worker-2 = {
              role = "worker";
              deployment.addr = "192.168.1.12:8080";
            };
          };

          # Control plane references peers by name
          services.procurator.control-plane = {
            enable = true;
            addr = "0.0.0.0:8080";
            peers = [ "worker-1" "worker-2" ];  # Automatically uses their addrs
          };
        }
      ];
    };
  };
}
```

### Option 2: Direct Address Configuration

```nix
{
  imports = [ procurator.nixosModules.procurator-worker ];

  services.procurator.worker = {
    enable = true;
    addr = "0.0.0.0:8080";
    masterAddr = "192.168.1.10:8080";  # Direct address
  };
}
```

## Worker Service Options

### `services.procurator.worker.enable`
- **Type:** boolean
- **Default:** `false`
- Enable the Procurator worker service.

### `services.procurator.worker.package`
- **Type:** package
- **Default:** `pkgs.procurator`
- The procurator package to use.

### `services.procurator.worker.hostname`
- **Type:** string
- **Default:** `config.networking.hostName`
- Hostname for this worker node.

### `services.procurator.worker.addr`
- **Type:** string
- **Example:** `"0.0.0.0:8080"`
- Address and port for the worker to bind to.

### `services.procurator.worker.master`
- **Type:** null or string
- **Default:** `null`
- **Example:** `"control-plane-1"`
- VM name from `cluster.vms` to use as the master control plane. Its `deployment.addr` will be used automatically. Takes precedence over `masterAddr` if both are set.

### `services.procurator.worker.masterAddr`
- **Type:** string
- **Default:** `""`
- **Example:** `"192.168.1.10:8080"`
- Direct address and port of the control plane master. Only used if `master` is null.

### `services.procurator.worker.user`
- **Type:** string
- **Default:** `"procurator-worker"`
- User account under which the worker runs.

### `services.procurator.worker.group`
- **Type:** string
- **Default:** `"procurator-worker"`
- Group under which the worker runs.

## Control Plane Service Options

### `services.procurator.control-plane.enable`
- **Type:** boolean
- **Default:** `false`
- Enable the Procurator control plane service.

### `services.procurator.control-plane.package`
- **Type:** package
- **Default:** `pkgs.procurator`
- The procurator package to use.

### `services.procurator.control-plane.hostname`
- **Type:** string
- **Default:** `config.networking.hostName`
- Hostname for this control plane node.

### `services.procurator.control-plane.addr`
- **Type:** string
- **Example:** `"0.0.0.0:8080"`
- Address and port for the control plane to bind to.

### `services.procurator.control-plane.peers`
- **Type:** list of strings
- **Default:** `[]`
- **Example:** `[ "worker-1" "worker-2" ]`
- List of VM names from `cluster.vms` to use as peers. Their `deployment.addr` will be used automatically. Takes precedence over `peersAddr` if both are set.

### `services.procurator.control-plane.peersAddr`
- **Type:** list of strings
- **Default:** `[]`
- **Example:** `[ "192.168.1.11:8080" "192.168.1.12:8080" ]`
- Direct list of peer control plane addresses for HA setup. Only used if `peers` is empty.

### `services.procurator.control-plane.user`
- **Type:** string
- **Default:** `"procurator-control-plane"`
- User account under which the control plane runs.

### `services.procurator.control-plane.group`
- **Type:** string
- **Default:** `"procurator-control-plane"`
- Group under which the control plane runs.

## Example: Combined Setup

```nix
{
  # Import both modules at once
  imports = [ procurator.nixosModules.default ];

  services.procurator = {
    control-plane = {
      enable = true;
      addr = "0.0.0.0:8080";
      peersAddr = [];
    };

    worker = {
      enable = false; # Usually not both on same machine
    };
  };
}
```

## Security

Both services include hardening:
- Run as dedicated unprivileged users
- `NoNewPrivileges=true`
- `PrivateTmp=true`
- `ProtectSystem=strict`
- `ProtectHome=true`
- State directory under `/var/lib/procurator-{worker,control-plane}`

## Systemd Integration

Services are managed via systemd:

```bash
# Control plane
sudo systemctl status procurator-control-plane
sudo systemctl restart procurator-control-plane
sudo journalctl -u procurator-control-plane -f

# Worker
sudo systemctl status procurator-worker
sudo systemctl restart procurator-worker
sudo journalctl -u procurator-worker -f
```
