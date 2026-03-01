# Worker — VM Management Daemon

## What

Manages cloud-hypervisor VM processes on a single host. Implements the `Worker` Cap'n Proto RPC interface (read status, list VMs, create VM, delete VM). One worker daemon runs per physical host in the cluster.

## Why

This is the execution layer — the component that actually runs VMs. The control plane decides *what* should run; the worker makes it happen by spawning cloud-hypervisor processes, managing their lifecycle, and reporting observed state back.

## Architecture

```
Control Plane / CLI
       │
  Cap'n Proto RPC (TCP)
       │
    Server (stateless RPC adapter)
       │  mpsc
    CommandSender → Node (message dispatch)
                      │  oneshot replies
                    VmManager<B: VmmBackend>
                      │
                    cloud-hypervisor processes
                      │  REST API over unix socket
                    VMs
```

- **Server** — Translates RPC calls to messages, sends them via `CommandSender`, awaits oneshot replies.
- **VmManager** — Single owner of all VM state. No locks — pure actor model. Generic over `VmmBackend` for testability.
- **VmmBackend trait** — `prepare()`, `spawn()`, `build_config()`. Production: `CloudHypervisorBackend`. Tests: `MockBackend`.
- **VM IDs** — UUIDv7 (time-ordered, sortable).
