# Worker Service — Networking TODO

## What was done

### Nix side (`service.nix`, `vmm.nix`)

| Change | File | Why |
|--------|------|-----|
| Added `kvm` and `netdev` groups to worker user | `service.nix` | `/dev/kvm` is owned by group `kvm`, `/dev/net/tun` by `netdev`. Without membership the worker can't open them. |
| Added `SupplementaryGroups` to systemd unit | `service.nix` | Systemd drops supplementary groups unless explicitly listed. Child processes (CH) inherit these groups. |
| Added `AmbientCapabilities` + `CapabilityBoundingSet` | `service.nix` | `CAP_NET_ADMIN` lets the worker create/delete TAP devices and attach them to bridges via netlink. `CAP_NET_RAW` is needed by CH for raw virtio-net I/O. Ambient caps propagate to child processes even with `NoNewPrivileges=true`. |
| Added `DevicePolicy = "closed"` + `DeviceAllow` | `service.nix` | Explicit allowlist of devices the service can access. Prevents future hardening (e.g. `PrivateDevices=true`) from accidentally blocking `/dev/net/tun`, `/dev/kvm`, `/dev/urandom`, `/dev/vhost-net`. |
| Added udev rules | `vmm.nix` | Guarantees `/dev/kvm` (group `kvm`), `/dev/net/tun` (group `netdev`), `/dev/vhost-net` (group `kvm`) have correct group ownership and `0660` permissions on all NixOS configurations. |

### Rust side (`cloud_hypervisor.rs`)

| Change | Why |
|--------|-----|
| Worker creates TAP via `ioctl(TUNSETIFF)` in `prepare()` | Worker owns the TAP lifecycle — create before CH starts, delete when VM is destroyed. CH just re-opens the persistent TAP by name. This avoids giving CH creation privileges. |
| TAP is created persistent (`TUNSETPERSIST`) | The fd is closed after creation. CH re-opens the same TAP by name via `--net tap=<name>`. |
| Added `create_tap_ioctl()` using `libc` | TAP creation is a `/dev/net/tun` ioctl, not a netlink operation. Requires `/dev/net/tun` rw access + `CAP_NET_ADMIN`. |
| Added `create_tap_device()` async wrapper | Runs ioctl in `spawn_blocking`, then brings the interface up via netlink. Handles stale TAP cleanup (crash recovery). |
| `ChProcess` now stores `tap_name: Option<String>` | On cleanup, the TAP is deleted via netlink. Without this, orphaned TAPs accumulate after VM deletion. |
| `delete_tap_device()` via netlink | Deletes TAP by name using `rtnetlink`. Called in `ChProcess::cleanup()` and as stale-device cleanup in `create_tap_device()`. |
| Added `libc` dependency | Needed for `ioctl`, `ifreq`, `IFNAMSIZ` types in TAP creation. |

---

## Remaining work

### 1. Orphaned TAP cleanup on worker restart
**What:** When the worker crashes or restarts, TAP devices from previous VMs remain in the kernel.
**Why:** Persistent TAPs survive process death. Without cleanup, the bridge accumulates stale interfaces.
**How:** On startup, enumerate all `pcr-*` interfaces via netlink and delete any that don't correspond to a known running VM. This should run in `VmManager::new()` or a startup hook.
**Priority:** High — without this, a crash + restart leaks TAPs until someone manually runs `ip link delete`.

### 2. `bridge_name` default should match `vmm.nix`
**What:** `CloudHypervisorConfig::default()` uses `bridge_name: Some("chbr0")` but `vmm.nix` creates `br0`.
**Why:** Mismatch means the Rust service won't find the bridge and will skip networking.
**How:** Change the default to `"br0"` or make it configurable via the worker config JSON. The NixOS module should pass the bridge name to the worker config.
**Priority:** High — currently a silent failure.

### 3. Bridge name in worker config JSON
**What:** The worker config file (`procurator-worker-config.json`) doesn't include the bridge name.
**Why:** The bridge name must match what `vmm.nix` creates. Hardcoding it in Rust is fragile.
**How:** Add `bridge_name` to the JSON config schema and wire it through `service.nix` → `configFile` → `CloudHypervisorConfig`.
**Priority:** Medium — blocking for multi-host deployments where bridge names differ.

### 4. MAC address generation
**What:** Each VM gets a random MAC from CH (no explicit MAC assigned).
**Why:** For DHCP reservations, monitoring, or audit logging, deterministic MACs are useful.
**How:** Generate a locally-administered MAC from the VM UUID (e.g. `02:xx:xx:xx:xx:xx` where `xx` = UUID bytes). Set it in `ChNetConfig.mac`.
**Priority:** Low — DHCP works fine with random MACs.

### 5. `ReadWritePaths` validation
**What:** `ProtectSystem = "strict"` makes the entire FS read-only except explicitly allowed paths.
**Why:** The worker writes to `/tmp/procurator/vms` (disk copies, sockets, logs). If any path is missed, the service fails with `EROFS`.
**How:** Verify that all writable paths are covered:
  - `/tmp/procurator/vms` — ✅ (in `ReadWritePaths`)
  - `/run/procurator-worker` — ✅ (via `RuntimeDirectory`)
  - `/var/lib/procurator-worker` — ✅ (via `StateDirectory`)
  - Nix store — read-only is fine (we only read kernel/initrd)
**Priority:** Medium — test by running the service and creating a VM.

### 6. `PrivateTmp` interaction with socket dir
**What:** `PrivateTmp = true` gives the service its own `/tmp` namespace.
**Why:** This means `/tmp/procurator/vms` inside the service is **not** the same as `/tmp/procurator/vms` on the host. Other tools (debug scripts, monitoring) won't see the sockets.
**How:** Either:
  - Move the socket/VM directory to `RuntimeDirectory` (`/run/procurator-worker/vms`) — recommended.
  - Or set `PrivateTmp = false` and keep `/tmp`.
**Priority:** High — affects whether you can `curl --unix-socket` the CH API for debugging.

### 7. Test the full flow on a NixOS machine
**What:** The Nix modules + Rust changes haven't been integration tested together.
**Why:** Capabilities, udev rules, group memberships, and device access all interact. A single mistake breaks the chain.
**How:** Deploy to a NixOS test VM:
  1. Enable `services.procurator.vmm` and `services.procurator.worker`
  2. Create a VM via the RPC API
  3. Verify: TAP created, attached to br0, VM boots, gets DHCP, can reach allowed domains
  4. Delete VM, verify TAP is cleaned up
**Priority:** Critical — nothing is validated until this passes.

### 8. Consider `nix` crate instead of raw `libc`
**What:** The TAP creation uses raw `libc::ioctl` with unsafe blocks.
**Why:** The `nix` Rust crate provides safe wrappers for Linux syscalls including ioctl.
**How:** Evaluate whether `nix::sys::ioctl` or `nix::net::if_` provides cleaner TAP creation. May reduce unsafe surface.
**Priority:** Low — current code works, this is a cleanliness improvement.
