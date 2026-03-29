# TAP Bridge Attach Permission Problem

## The Problem

The worker runs `ip link set <tap> master chbr0 up` to attach a VM's TAP
device to the host bridge. This fails with `RTNETLINK answers: Operation
not permitted`.

## Why It Fails

Linux file capabilities (`setcap cap_net_admin+ep`) are attached to a
**binary file** and apply to the **process that binary creates**. They do
**not** propagate to child processes.

```
worker binary (has CAP_NET_ADMIN via setcap)
  └─ spawns `ip link set ...` as child process
       └─ ip binary does NOT have CAP_NET_ADMIN → EPERM
```

The worker process itself has `CAP_NET_ADMIN` — any netlink syscall made
*from within the worker process* would succeed. But `Command::new("ip")`
spawns a **new** process, which starts fresh without capabilities.

Cloud Hypervisor doesn't have this problem because it creates TAP devices
via its own netlink syscalls — it never shells out to `ip`.

## Options

### Option A: Direct netlink from Rust (chosen)

Replace the `ip link set` subprocess call with direct netlink syscalls
from the worker process using the `rtnetlink` crate (async, tokio-native).

```rust
// Instead of:
Command::new("ip").args(["link", "set", &tap, "master", &bridge, "up"])

// Do:
let (conn, handle, _) = rtnetlink::new_connection()?;
tokio::spawn(conn);
let tap_idx = handle.link().get().match_name(tap).execute().try_next().await?;
handle.link().set(tap_idx).master(bridge_idx).up().execute().await?;
```

**Pros:**
- Cleanest solution — no extra binaries, scripts, or wrappers
- The worker already has `CAP_NET_ADMIN`; netlink syscalls from within the
  process work immediately
- No capability inheritance issues
- Async-native (tokio), no blocking subprocess
- Removes dependency on `ip` being in `$PATH`

**Cons:**
- Adds `rtnetlink` + `netlink-packet-route` crate dependencies
- Slightly more complex Rust code than a one-liner shell-out

### Option B: Capped helper script/binary

Create a small setuid or capability-enabled helper binary that does only
the bridge attach:

```
/var/lib/procurator/bin/pcr-attach-tap <tap> <bridge>
```

The setup script copies `ip` (or a wrapper) there and gives it
`CAP_NET_ADMIN`.

**Pros:**
- Simple concept — worker calls a privileged helper
- No new Rust dependencies

**Cons:**
- Another binary to install, copy, and cap — more moving parts in setup
- Setuid/capped binaries are a security surface
- Still shelling out to a subprocess

### Option C: Run `ip` via ambient capabilities

Use `capsh` or `prctl(PR_CAP_AMBIENT)` to set ambient capabilities so
they inherit to child processes.

**Pros:**
- Worker keeps using `Command::new("ip")`

**Cons:**
- Fragile — ambient caps require specific conditions (must be in permitted
  and inheritable sets, caller must have `CAP_SETPCAP`)
- Hard to get right in a Nix wrapper context
- Not widely used or well-tested

### Option D: Run the `ip` command via `sudo`

The setup script adds a sudoers rule allowing the worker user to run
`ip link set ... master chbr0` without a password.

**Pros:**
- Simple, well-understood mechanism
- No new Rust code

**Cons:**
- Requires sudoers configuration (another setup step)
- `sudo` in a hot path feels wrong for a daemon
- Tighter coupling to the host's auth setup

---

## Very simple summary (for quick mental model)

### What is a TAP?

A TAP is a "fake" network cable that lives inside the Linux kernel. When a
virtual machine sends or receives Ethernet frames, they come out of the
host via a TAP interface (named like `pcr-019cc925-a1` in our logs). The
hypervisor (cloud-hypervisor) opens `/dev/net/tun` and asks the kernel to
give it a TAP device. That device behaves exactly like a real Ethernet
port; you can plug it into a bridge, assign an IP, etc. The TAP is the
host-side end of the VM's virtual network card.

### What is a bridge?

A bridge (e.g. `chbr0`) is like a software Ethernet switch: you "plug"
several interfaces into it and they can talk to each other. VMs connect
their TAP devices to the bridge so they can communicate with the outside
world. The bridge itself gets an IP address (192.168.249.1) and the host
uses NAT to let VMs reach the Internet.

### Why do we need CAP_NET_ADMIN?

To create TAP devices and to change networking configuration you need
special privileges. Instead of running the entire worker as root, we give
only the binaries that actually touch networking the single capability
`CAP_NET_ADMIN`. This is like saying "you are allowed to change network
settings but nothing else." Both `cloud-hypervisor` and our `worker`
need this capability because they each do `ip`/netlink operations.

### What went wrong?

When we start a VM the worker tells cloud-hypervisor to create a TAP
device. CH creates the TAP (it has the capability and runs in the
networking-privileged copy). After CH returns, the worker needs to attach
the new TAP to the bridge. The worker does this by running the external
program `ip link set ...`. Even though the worker process itself has the
CAP_NET_ADMIN capability, the new `ip` process **does not**. Linux
capabilities attach to the binary, not to the user or the parent process,
and they are not inherited by children. In short: worker → spawns `ip` →
child has no privileges → `ip` fails with "Operation not permitted."


---
