# Service Modules — NixOS Usage

## What

NixOS modules under `nix/modules/` for the five Procurator services. Each module generates the service's config file (TOML/JSON) and one hardened systemd unit; the binaries come from `nix/flake/packages.nix`.

## Modules

| Module (`services.procurator.<name>`) | Unit                    | Binary        | Config file                       |
| ------------------------------------- | ----------------------- | ------------- | --------------------------------- |
| `control-plane`                       | `procurator-control-plane` | `bin/control_plane` | JSON (`procurator-control-plane-config.json`) |
| `worker`                              | `procurator-worker`     | `bin/worker`  | JSON (`procurator-worker-config.json`) |
| `cache`                               | `procurator-cache`      | `bin/cache`   | TOML (`procurator-cache.toml`)    |
| `ci`                                  | `procurator-ci`         | `bin/ci_service` | TOML (`procurator-ci.toml`)    |
| `repohub`                             | `procurator-repohub`    | `bin/repohub` | TOML (`procurator-repohub.toml`)  |
| `gitServer`                           | (no unit; users + tmpfiles only) | —    | —                                 |
| `vmm`                                 | (host bridge/networking for worker VMs) | — | —                          |

Every module follows the same shape: `package` (defaults to the built crate), `addr`, `user`/`group`, `extraArgs`, plus service-specific options. Config keys mirror the Rust structs (`deny_unknown_fields` in Rust keeps them in sync — a typo'd module config fails at startup).

## Quick start (consumer flake)

```nix
{
  inputs.procurator.url = "github:crocoder-dev/procurator";

  outputs = { self, procurator, nixpkgs, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        # ONE overlay: puts pkgs.cache, pkgs.worker, pkgs.ci_service,
        # pkgs.control_plane, pkgs.repohub ... on your pkgs set so the
        # module package defaults resolve.
        procurator.overlays.default
        procurator.nixosModules.procurator
        ({ ... }: {
          services.procurator.enable = true;

          services.procurator.control-plane = {
            enable = true;
            addr = "127.0.0.1:8080";
            workers = [ "worker-1" ];  # references cluster.vms or set workersAddr
          };

          services.procurator.worker = {
            enable = true;
            workerId = "worker-1";
            masterAddr = "192.168.1.10:8080";  # or master = "control-plane-1"
            proxyJwtHs256Secret = "<required: strong secret when proxy binds non-loopback>";
          };

          services.procurator.cache.enable = true;   # 0.0.0.0:8081, mTLS via tls.* options
          services.procurator.ci.enable = true;      # 0.0.0.0:8082
          services.procurator.repohub.enable = true; # 0.0.0.0:8083
          services.procurator.gitServer.enable = true;
        })
      ];
    };
  };
}
```

## Ports

Control plane (Cap'n Proto) 8080 · cache 8081 · CI 8082 · repohub 8083 · worker RPC 8080-default (override on co-hosted hosts) · worker TLS proxy 8443.

## Tests

All pure eval — no builds, no VMs:

```sh
# per-module fast tests (option wiring, generated configs, assertions)
nix eval --json -f ./nix/tests/worker-service-fast.nix --apply 'f: f {}'
# (same for cache-fast / ci-service-fast / repohub-fast / control-plane-fast)

# whole-set: all five services composed into one real NixOS system
nix build .#checks.x86_64-linux.services-system-eval

# everything
nix flake check
```
