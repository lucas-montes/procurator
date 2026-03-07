# Networking Glossary & Design

Reference document for VM networking concepts used in Procurator.

---

## Glossary

### TAP device

A **TAP** (network **T**un/**A**ccess **P**oint) is a **virtual network
interface** that exists only in the Linux kernel — there's no physical cable
or WiFi radio behind it. It looks and behaves exactly like a real Ethernet
port (has a MAC address, carries Ethernet frames), but instead of being
connected to hardware, it's connected to a **userspace program** (in our
case, Cloud Hypervisor).

**Analogy:** Imagine plugging a virtual Ethernet cable from the VM's network
card into the host's network stack. The TAP device is the host-side plug of
that cable.

**How it works:**
1. A program opens `/dev/net/tun` and asks the kernel: "create a TAP device
   named `vmtap0`".
2. The kernel creates the virtual interface. It shows up in `ip link` just
   like `eth0` or `wlan0`.
3. When the VM sends a network packet, Cloud Hypervisor writes it to the TAP.
   The kernel receives it on `vmtap0` as if a real NIC sent it.
4. When the host sends a packet to `vmtap0`, Cloud Hypervisor reads it and
   delivers it to the VM's virtual NIC.

**Who creates it:** Cloud Hypervisor creates the TAP device automatically when
you include a `net` section in the VM config. If you pass `tap: "mytap"`, it
uses/creates that specific name. If you omit the tap name, CH auto-names it
`vmtapN`.

**Permissions:** Creating a TAP requires `CAP_NET_ADMIN` capability. The
cloud-hypervisor binary gets this via `setcap cap_net_admin+ep`.

### Bridge

A **bridge** (e.g., `chbr0`) is a **virtual network switch**. It connects
multiple network interfaces together so they can talk to each other at Layer 2
(Ethernet frames).

**Analogy:** Think of a physical Ethernet switch with 8 ports. A bridge is
the same thing, but virtual. You "plug" TAP devices (VM virtual cables) into
it, and they can all talk to each other and to the host.

**Why we need it:**
- Multiple VMs each have their own TAP device.
- The bridge connects all TAPs into one network segment.
- The host can also have an IP on the bridge (e.g., `192.168.249.1`), acting
  as the gateway for all VMs.

**How it works:**
1. The host creates a bridge: `ip link add chbr0 type bridge`.
2. Each VM's TAP gets attached: `ip link set vmtap0 master chbr0`.
3. The bridge gets an IP: `ip addr add 192.168.249.1/24 dev chbr0`.
4. VMs on the bridge can now reach each other and the host.

### NAT (Network Address Translation) / Masquerade

**NAT** lets VMs access the internet through the host's IP address. Without it,
VMs have private IPs (e.g., `192.168.249.x`) that the internet doesn't know
how to route back to.

**Analogy:** NAT is like a receptionist at a company. When an employee (VM)
calls an outside number, the receptionist (host) makes the call from the
company's main phone number, remembers which employee asked, and routes the
response back to them.

**How it works:**
1. VM sends a packet to `google.com` via the bridge → host.
2. The host's iptables/nftables MASQUERADE rule rewrites the source IP from
   `192.168.249.10` (VM) to the host's public IP.
3. The response comes back to the host, which rewrites the destination back
   to `192.168.249.10` and sends it through the bridge to the VM.

**NixOS:** `networking.nat.enable = true; networking.nat.internalInterfaces = [ "chbr0" ];`

### DHCP (Dynamic Host Configuration Protocol)

**DHCP** is how a device automatically gets an IP address when it connects to
a network. Instead of manually configuring `192.168.249.10` inside the VM,
the VM sends a broadcast "I need an IP!" and a DHCP server responds with one.

**In our setup:** dnsmasq runs on the host, listening on the bridge. When a VM
boots and its virtio-net interface comes up, systemd-networkd (inside the VM)
sends a DHCP request. dnsmasq assigns an IP from the configured range
(e.g., `192.168.249.10` to `192.168.249.50`).

### dnsmasq

**dnsmasq** is a lightweight program that provides both **DHCP** and **DNS**
services. In our setup it runs on the host, bound to the bridge interface.

**What it does:**
1. **DHCP server** — hands out IP addresses to VMs when they boot.
2. **DNS forwarder** — VMs use the bridge IP as their DNS server. dnsmasq
   forwards DNS queries to the host's upstream DNS servers (e.g., your
   router or `8.8.8.8`).

**Why dnsmasq and not something else:** It's tiny (single binary), does both
DHCP + DNS, is widely used in QEMU/libvirt setups, and is available in nixpkgs.

### virtio-net

**virtio-net** is a **paravirtualized network card**. Instead of emulating a
real hardware NIC (like Intel e1000), virtio-net is a simplified, high-performance
interface designed specifically for VMs. Both the host (Cloud Hypervisor) and
guest (Linux kernel) know it's virtual, so they can skip the overhead of
pretending to be real hardware.

**Guest side:** The Linux kernel in the VM loads the `virtio_net` driver
(already in our initrd via `boot.initrd.availableKernelModules`). It shows
up as an Ethernet interface (e.g., `enp0s3` or `eth0`).

**Host side:** Cloud Hypervisor creates a TAP device and connects the
virtio-net device to it. Packets flow: VM ↔ virtio-net ↔ TAP ↔ bridge ↔
NAT ↔ internet.

### nftables

**nftables** is the modern Linux firewall framework (successor to iptables).
We use it for **domain allowlisting** — restricting which internet domains
a VM can reach.

**How domain allowlisting works:**
1. Resolve allowed domains (e.g., `google.com`) to IP addresses.
2. Create nftables rules: allow DNS (port 53), allow those IPs, drop
   everything else.
3. VMs can only reach the allowed domains.

### CAP_NET_ADMIN

Linux **capabilities** are fine-grained permissions that replace the old
"root or nothing" model. `CAP_NET_ADMIN` specifically grants permission to:
- Create/delete network interfaces (TAP devices)
- Modify network configurations (IP addresses, routes)
- Manage bridges

**Instead of running as root,** we give the cloud-hypervisor binary just
this one capability: `setcap cap_net_admin+ep /path/to/cloud-hypervisor`.
This is the minimum privilege needed for CH to create its own TAP device.

### setcap

`setcap` is the command that attaches Linux capabilities to a binary file.
```
sudo setcap cap_net_admin+ep ./cloud-hypervisor
```
The `+ep` means:
- `e` = **effective**: the capability is active when the program runs
- `p` = **permitted**: the capability is allowed (can be activated)

After this, any user can run `cloud-hypervisor` and it will have
`CAP_NET_ADMIN` — no `sudo` needed for the worker process itself.

---

## The Full Picture: How VM Networking Works

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│     VM 1     │     │     VM 2     │     │     VM N     │
│  virtio-net  │     │  virtio-net  │     │  virtio-net  │
│  (guest NIC) │     │  (guest NIC) │     │  (guest NIC) │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │ packets            │                    │
┌──────▼───────┐     ┌──────▼───────┐     ┌──────▼───────┐
│   vmtap0     │     │   vmtap1     │     │   vmtapN     │
│  (TAP device)│     │  (TAP device)│     │  (TAP device)│
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       └────────────┬───────┴────────────────────┘
                    │
            ┌───────▼───────┐
            │    chbr0      │  ← bridge (virtual switch)
            │ 192.168.249.1 │  ← gateway IP
            └───────┬───────┘
                    │
            ┌───────▼───────┐
            │  NAT/masquerade│  ← source IP rewriting
            └───────┬───────┘
                    │
            ┌───────▼───────┐
            │  host NIC     │  ← real network (eth0/wlan0)
            │  (internet)   │
            └───────────────┘
```

**Data flow for a VM pinging google.com:**
1. VM's `curl google.com` → DNS query to `192.168.249.1` (dnsmasq)
2. dnsmasq resolves `google.com` → `142.250.x.x`, returns to VM
3. VM sends HTTP packet to `142.250.x.x` via its virtio-net card
4. Cloud Hypervisor writes the packet to the TAP device
5. TAP → bridge → NAT rewrites source to host's IP → out to internet
6. Response comes back → NAT → bridge → TAP → CH → VM

---

## How Other Tools Handle This

### Docker (containers)

Docker uses the same building blocks (bridge + veth pairs + NAT + iptables):
- Creates a bridge (`docker0`) at daemon startup
- For each container, creates a **veth pair** (similar to TAP, but for
  containers instead of VMs): one end goes in the container's network
  namespace, the other connects to `docker0`
- NAT/masquerade for outbound internet access
- Embedded DNS server (instead of dnsmasq) for service discovery
- `dockerd` runs as **root** and manages all networking itself

**Key difference from VMs:** Containers use veth pairs (Layer 2, kernel
namespace trick). VMs use TAP devices (the hypervisor process reads/writes
raw Ethernet frames through `/dev/net/tun`).

### Kubernetes (container orchestration)

Kubernetes itself does NOT implement networking. It defines a **CNI**
(Container Network Interface) spec and delegates to plugins:
- **Flannel:** Simple overlay network using VXLAN tunnels between hosts
- **Calico:** BGP-based routing, no overlay (higher performance)
- **Cilium:** eBPF-based, no iptables (newest, fastest)

Each pod gets its own IP. Cross-host networking is handled by the CNI plugin
(tunnels, routing tables, etc.). This is much more complex than what we need.

### QEMU/libvirt (traditional VMs)

The most direct comparison. libvirt manages QEMU VMs and does:
- Creates a bridge (`virbr0`) with a DHCP/DNS server (dnsmasq)
- For each VM, creates a TAP device and attaches it to the bridge
- NAT/masquerade via iptables
- Uses helper scripts (`/etc/qemu/bridge.conf`, `qemu-bridge-helper`)
  so the TAP creation can happen without root

**This is essentially what we do,** just with Cloud Hypervisor instead of QEMU.

### Firecracker (lightweight VMs)

Firecracker (AWS Lambda/Fargate) takes a simpler approach:
- Requires a **pre-created TAP device** — Firecracker never creates TAPs itself
- The orchestrator (host agent) creates TAPs, bridges, and iptables rules
- Uses `jailer` for security (cgroups, seccomp, chroot)
- Networking setup is 100% external to the VMM

### Summary of approaches

| Tool          | Who creates TAP/veth? | Bridge    | DHCP/DNS  | NAT         |
|---------------|----------------------|-----------|-----------|-------------|
| Docker        | dockerd (root)       | docker0   | embedded  | iptables    |
| QEMU/libvirt  | libvirt/helper       | virbr0    | dnsmasq   | iptables    |
| Firecracker   | external orchestrator| external  | external  | external    |
| Cloud Hyp.    | CH itself (CAP_NET_ADMIN) | external | external | external |
| **Procurator**| **CH (CAP_NET_ADMIN)**| **chbr0 (NixOS)** | **dnsmasq (NixOS)** | **nftables (NixOS)** |

---

## Procurator's Approach

We follow the **QEMU/libvirt pattern** adapted for NixOS, with **defense in
depth** for domain allowlisting: guest-side nftables is the primary filter,
host-side nftables is a secondary production layer.

### Guest-side filtering (primary — always active)

When `allowedDomains` is non-empty, the VM itself enforces restrictions.
A systemd service (`vm-domain-firewall.service`) runs at boot **before**
the workload, resolves each domain → IPs via `getent`, and installs nftables
output rules:

- **Allow:** loopback, DNS (53), DHCP (67/68), established/related,
  resolved IPs of each allowed domain
- **Drop:** all other outbound traffic

This works regardless of host setup — even without a bridge, NAT, or NixOS
host module, the guest self-enforces. Empty `allowedDomains` = no firewall.

**Key file:** `nix/lib/image/vm-module.nix`

### Host networking setup

Two paths to set up the host bridge/NAT/DHCP:

#### Dev / `nix run` path (automatic with sudo)

The worker wrapper (`nix/flake/apps.nix`) detects whether host networking
is set up. If not, it runs a one-time `sudo` setup script that:

1. Creates bridge `chbr0` with IP `192.168.249.1/24`
2. Enables `ip_forward` if not already on
3. Adds iptables masquerade rule for NAT
4. Copies the CH binary to `/run/pcr/cloud-hypervisor` and sets
   `CAP_NET_ADMIN` via `setcap`
5. Starts dnsmasq on the bridge for DHCP + DNS

The script is idempotent — safe to run multiple times. After the first run,
subsequent `nix run ./nix#worker` invocations skip sudo entirely.

#### Production / NixOS module path

`nix/modules/host/default.nix` provides the same infrastructure declaratively:
bridge, dnsmasq, NAT, `security.wrappers` for CAP_NET_ADMIN, plus host-side
nftables domain rules as a second filtering layer.

### Per-VM networking

1. **CH creates TAP per VM** — `build_config()` sets `net: [{tap: "pcr-<id>"}]`.
   CH creates the TAP at `vm.create()` time. Requires `CAP_NET_ADMIN`.
2. **Worker attaches TAP to bridge** — `attach_network()` runs
   `ip link set <tap> master chbr0 up` between `create()` and `boot()`.
3. **Guest gets IP via DHCP** — systemd-networkd in the VM requests DHCP
   from dnsmasq on the bridge.
4. **Guest firewall activates** — `vm-domain-firewall.service` resolves
   allowed domains and installs nftables rules before the workload starts.

### Why defense in depth?

- **Guest-side** catches everything even if host is misconfigured
- **Host-side** (NixOS module) prevents a compromised guest from bypassing
  restrictions — nftables rules on the host's forward chain can't be
  modified from inside the VM
- The two layers are independent and additive

### Capability strategy

Instead of running the worker as root, we use Linux capabilities:
```
setcap cap_net_admin+ep /path/to/cloud-hypervisor
```
This gives CH the minimum privilege needed to create TAP devices. The worker
process itself runs as a normal user.

- **Dev:** wrapper script copies CH binary to `/run/pcr/` and applies `setcap`
- **Production:** `security.wrappers` creates `/run/wrappers/bin/cloud-hypervisor`
