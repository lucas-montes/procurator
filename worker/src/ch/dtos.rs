use std::net::Ipv4Addr;
use std::ops::Not;

use serde::Serialize;
use tracing::debug;

/// Convert a dotted-quad netmask (e.g. `255.0.0.0`) to a CIDR prefix length
/// (e.g. `8`). Used by `VmConfigRef::apply_runtime_network` to emit the
/// standalone `procurator.pfx=<n>` token; the in-image `procurator-netcfg`
/// unit recombines it with the IP (`ip addr add <ip>/<pfx>`) before applying.
fn netmask_to_prefix(mask: Ipv4Addr) -> u8 {
    // SAFETY: count_ones on a u32 is always 0..=32, which fits in u8.
    u8::try_from(u32::from_be_bytes(mask.octets()).count_ones())
        .expect("count_ones of IPv4 netmask fits in u8")
}

//TODO: we should use more types to know if we are receiving it from the server or using it for the client
// meaning that it contains the latest and updated config

// ── CH API-facing DTOs ────────────────────────────────────────────────────────
// These types serialise to the JSON body sent to the cloud-hypervisor REST API.
// All string fields borrow from the capnp message to avoid extra allocations.
// Optional CH fields use Option<T> + skip_serializing_if so they are omitted
// entirely from JSON when absent (CH treats missing fields as their defaults).

/// Top-level VM configuration sent to POST /api/v1/vm.create.
/// Mirrors `VmConfig` in ch.capnp / cloud-hypervisor.yaml.
#[derive(Debug, Clone, Serialize)]
pub struct VmConfigRef<'a> {
    cpus: CpusConfigRef,
    memory: MemoryConfigRef,
    payload: PayloadConfigRef<'a>,
    disks: Vec<DiskConfigRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    net: Option<Vec<NetConfigRef<'a>>>,
    console: ConsoleConfigRef<'a>,
    serial: ConsoleConfigRef<'a>,
}

/// Maps to `CpusConfig` in ch.capnp.
#[derive(Debug, Clone, Serialize)]
pub struct CpusConfigRef {
    boot_vcpus: u32,
    max_vcpus: u32,
}

/// Maps to `MemoryConfig` in ch.capnp. `size` is required by the CH API.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryConfigRef {
    size: u64,
}

/// Maps to `DiskConfig` in ch.capnp. The worker overrides `path` at runtime
/// with the per-VM writable copy.
///
/// `image_type` is forwarded verbatim to CH as `image_type` in the JSON body.
/// CH would otherwise probe the disk by magic bytes, which has misidentified
/// NixOS raw images as qcow on recent releases — explicit is safer.
#[derive(Debug, Clone, Serialize)]
pub struct DiskConfigRef<'a> {
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_type: Option<&'a str>,
}

/// Maps to `NetConfig` in ch.capnp.
///
/// IMPORTANT: CH's `ip` / `mask` fields configure the **host-side TAP device**,
/// not the guest. Because our TAPs are always enslaved to the host bridge
/// (`br0`), the TAP itself must NOT carry an L3 address — the bridge owns it.
/// We therefore send only `tap` and `mac`. The guest receives its IP via the
/// `procurator.ip=` / `procurator.gw=` / `procurator.pfx=` cmdline tokens
/// appended by `factory::append_runtime_tokens`, which the in-image
/// `procurator-netcfg` systemd unit applies to `eth0` before `network.target`.
#[derive(Debug, Clone, Serialize)]
pub struct NetConfigRef<'a> {
    tap: &'a str,
    mac: &'a str,
}

impl<'a> NetConfigRef<'a> {
    pub fn new(tap: &'a str, mac: &'a str) -> Self {
        Self { tap, mac }
    }
}

/// Maps to `ConsoleConfig` in ch.capnp.
/// Used for both `console` and `serial` in `VmConfigRef`.
/// `mode` is required; `file` is only meaningful when `mode = "File"`.
#[derive(Debug, Clone, Serialize)]
pub struct ConsoleConfigRef<'a> {
    mode: &'a str, //TODO: this could be an enum I think
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
}

impl<'a> VmConfigRef<'a> {
    /// Apply every per-VM mutation in a single pass and return the finalised
    /// config ready to POST to Cloud Hypervisor.
    ///
    /// Mutations performed:
    ///
    /// 1. **Cmdline** — append three independent runtime tokens:
    ///    ```text
    ///     <base> procurator.ip=<ip> procurator.gw=<gw> procurator.pfx=<prefix>
    ///    ```
    ///    This is a contract with the in-image `procurator-netcfg` systemd
    ///    unit (see `nix/lib/diskVm.nix`), which parses `/proc/cmdline` and
    ///    refuses to start unless **all three** tokens are present.
    ///    Combining IP and prefix into a single `procurator.ip=<ip>/<pfx>`
    ///    (CIDR form) breaks that guard — the unit exits 1, leaving `eth0`
    ///    down and the VM unreachable. Mirrors the dev launcher in
    ///    `nix/examples/.../flake.nix` byte-for-byte.
    ///
    ///    We do NOT use the stock Linux `ip=<...>:::off` boot parameter:
    ///    NixOS ships its kernel with `CONFIG_IP_PNP=n`, so it is silently
    ///    ignored. Dotted namespaced keys also suppress the kernel's
    ///    "Unknown kernel command line parameters" warning.
    ///
    /// 2. **Disk** — redirect the first disk's `path` to the per-VM writable
    ///    copy under `<runtime_dir>/<vm_id>/root.img`. The `image_type` set
    ///    by `TryFrom` (defaults to `"raw"`) is preserved.
    ///
    /// 3. **Net** — install the (tap, mac) pair allocated for this VM. Only
    ///    `tap` and `mac` are sent to CH; the guest IP is applied inside the
    ///    VM by `procurator-netcfg`, never by CH's `NetConfig` (which would
    ///    assign an IP to the host-side TAP — wrong, the TAP is a bridge
    ///    slave and must stay L3-less).
    ///
    /// 4. **Console / serial** — console off (CH spam goes nowhere), serial
    ///    redirected to the per-VM `serial.log`.
    pub fn finalize_for_runtime(
        mut self,
        ip: Ipv4Addr,
        gateway: Ipv4Addr,
        mask: Ipv4Addr,
        writable_disk_path: &'a str,
        serial_log_path: &'a str,
        network: NetConfigRef<'a>,
    ) -> Self {
        // ── 1. Cmdline ───────────────────────────────────────────────
        // Mutate the existing `String` in place to reuse its allocation.
        use std::fmt::Write as _;
        let prefix = netmask_to_prefix(mask);
        let cmdline = &mut self.payload.cmdline;
        // Drop trailing whitespace/newline from the image-baked cmdline so
        // we don't end up with `init=...\n procurator.ip=...`. capnp
        // already rejects empty cmdlines, so this only trims stray bytes.
        let trimmed_len = cmdline.trim_end().len();
        cmdline.truncate(trimmed_len);
        // `write!` to a `String` is infallible; only allocates if the
        // appended tokens push past the current capacity (typically once).
        let _ = write!(
            cmdline,
            " procurator.ip={ip} procurator.gw={gateway} procurator.pfx={prefix}"
        );

        // ── 2. Disk ──────────────────────────────────────────────────
        if let Some(first_disk) = self.disks.first_mut() {
            first_disk.path = writable_disk_path;
            // image_type from capnp is preserved (defaulted to "raw").
        } else {
            self.disks.push(DiskConfigRef {
                path: writable_disk_path,
                image_type: Some("raw"),
            });
        }

        // ── 3. Net ───────────────────────────────────────────────────
        let tap = network.tap;
        let mac = network.mac;
        self.net = Some(vec![network]);

        // ── 4. Console / serial ──────────────────────────────────────
        self.console = ConsoleConfigRef {
            mode: "Off",
            file: None,
        };
        self.serial = ConsoleConfigRef {
            mode: "File",
            file: Some(serial_log_path),
        };

        debug!(
            cmdline = %self.payload.cmdline,
            disk = %self.disks[0].path,
            disk_image_type = ?self.disks[0].image_type,
            tap,
            mac,
            console_mode = self.console.mode,
            serial_mode = self.serial.mode,
            serial_file = ?self.serial.file,
            "VmConfigRef finalised for runtime"
        );

        self
    }
}

// ── Capnp reader → Rust struct DTOs ──────────────────────────────────────────
// These types deserialise from capnp messages coming from the control plane.

#[derive(Debug, Clone, Serialize)]
pub struct PayloadConfigRef<'a> {
    kernel: &'a str,
    // Image-specific base cmdline received over capnp (originally written by
    // the flake's `artifacts` derivation into `$out/cmdline`). The worker
    // overwrites this field in `factory::create_vm` by calling
    // `VmConfigRef::set_cmdline` with the base + runtime tokens appended.
    cmdline: String,
    initramfs: &'a str,
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn require_non_empty<'a>(value: &'a str, field: &'static str) -> Result<&'a str, capnp::Error> {
    if value.is_empty() {
        Err(capnp::Error::failed(format!(
            "required field '{field}' is empty"
        )))
    } else {
        Ok(value)
    }
}

#[derive(Debug)]
pub struct CreateVmSpecRef<'a> {
    vm_config: VmConfigRef<'a>,
}

impl<'a> CreateVmSpecRef<'a> {
    pub fn vm_config(self) -> VmConfigRef<'a> {
        self.vm_config
    }

    pub fn kernel(&self) -> &str {
        self.vm_config.payload.kernel
    }

    pub fn initramfs(&self) -> &str {
        self.vm_config.payload.initramfs
    }

    pub fn root_disk(&self) -> &str {
        self.vm_config.disks[0].path
    }
}

impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>>
    for CreateVmSpecRef<'a>
{
    type Error = capnp::Error;

    fn try_from(
        reader: commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>,
    ) -> Result<Self, Self::Error> {
        let reader = reader.get_spec()?;
        let cpus = reader.get_cpus()?;
        let memory = reader.get_memory()?;
        let payload = reader.get_payload()?;

        let disks_reader = reader.get_disks()?;
        if disks_reader.is_empty() {
            return Err(capnp::Error::failed(
                "vm config must contain at least one disk".to_string(),
            ));
        }
        let mut disks: Vec<DiskConfigRef<'_>> = Vec::with_capacity(disks_reader.len() as usize);
        for disk in disks_reader {
            // Default `image_type` to "raw" when the control plane omits it.
            // The reference image produced by `nix/lib/diskVm.nix` is a raw
            // ext4 image (`make-disk-image.nix` with `format = "raw"`).
            // Without an explicit type, CH may misidentify it as qcow.
            let image_type = match disk.get_image_type()?.to_str()? {
                "" => Some("raw"),
                other => Some(other),
            };
            disks.push(DiskConfigRef {
                path: require_non_empty(disk.get_path()?.to_str()?, "disk.path")?,
                image_type,
            });
        }

        // NOTE: the capnp `NetConfig` still carries legacy `ip` / `mask` fields
        // for schema backward-compatibility, but we deliberately do not read
        // them here. The worker assigns the guest IP via the
        // `procurator.ip=`/`procurator.gw=`/`procurator.pfx=` cmdline tokens
        // (see `factory::append_runtime_tokens`). The TAP itself must not
        // carry an L3 address because it is enslaved to the host bridge.
        let net = match reader.get_net() {
            Ok(net_reader) if net_reader.is_empty().not() => {
                let mut nets = Vec::with_capacity(net_reader.len() as usize);
                for net_cfg in net_reader {
                    nets.push(NetConfigRef {
                        tap: require_non_empty(net_cfg.get_tap()?.to_str()?, "net.tap")?,
                        mac: require_non_empty(net_cfg.get_mac()?.to_str()?, "net.mac")?,
                    });
                }
                Some(nets)
            }
            _ => None,
        };

        let console_reader = reader.get_console()?;
        let console = ConsoleConfigRef {
            mode: require_non_empty(console_reader.get_mode()?.to_str()?, "console.mode")?,
            file: non_empty(console_reader.get_file()?.to_str()?),
        };

        let serial_reader = reader.get_serial()?;
        let serial = ConsoleConfigRef {
            mode: require_non_empty(serial_reader.get_mode()?.to_str()?, "serial.mode")?,
            file: non_empty(serial_reader.get_file()?.to_str()?),
        };

        Ok(Self {
            vm_config: VmConfigRef {
                cpus: CpusConfigRef {
                    boot_vcpus: cpus.get_boot_vcpus(),
                    max_vcpus: cpus.get_max_vcpus(),
                },
                memory: MemoryConfigRef {
                    size: memory.get_size(),
                },
                payload: PayloadConfigRef {
                    kernel: require_non_empty(payload.get_kernel()?.to_str()?, "payload.kernel")?,
                    // Base cmdline from the capnp message (produced by the
                    // flake's `artifacts` derivation). The worker appends
                    // runtime tokens and overwrites this field before POSTing
                    // to Cloud Hypervisor.
                    cmdline: require_non_empty(
                        payload.get_cmdline()?.to_str()?,
                        "payload.cmdline",
                    )?
                    .to_owned(),
                    initramfs: require_non_empty(
                        payload.get_initramfs()?.to_str()?,
                        "payload.initramfs",
                    )?,
                },
                disks,
                net,
                console,
                serial,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConsoleConfigRef, CpusConfigRef, CreateVmSpecRef, DiskConfigRef, MemoryConfigRef,
        NetConfigRef, PayloadConfigRef, VmConfigRef,
    };

    fn make_vm_config<'a>(
        disk_path: &'a str,
        net: Option<Vec<NetConfigRef<'a>>>,
    ) -> VmConfigRef<'a> {
        VmConfigRef {
            cpus: CpusConfigRef {
                boot_vcpus: 2,
                max_vcpus: 2,
            },
            memory: MemoryConfigRef {
                size: 512 * 1024 * 1024,
            },
            payload: PayloadConfigRef {
                kernel: "/path/to/kernel",
                cmdline: "console=ttyS0 root=/dev/vda init=/nix/store/fake/init".to_string(),
                initramfs: "/path/to/initramfs",
            },
            disks: vec![DiskConfigRef {
                path: disk_path,
                image_type: Some("raw"),
            }],
            net,
            console: ConsoleConfigRef {
                mode: "Pty",
                file: None,
            },
            serial: ConsoleConfigRef {
                mode: "Tty",
                file: None,
            },
        }
    }

    #[test]
    fn finalize_for_runtime_overrides_disk_serial_console_and_enables_net() {
        use std::net::Ipv4Addr;
        let config = make_vm_config("/original/disk.raw", None);

        let finalized = config.finalize_for_runtime(
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(255, 0, 0, 0),
            "/runtime/overlay.qcow2",
            "/var/log/serial.log",
            NetConfigRef::new("tap0", "02:00:00:00:00:01"),
        );

        assert_eq!(finalized.disks.len(), 1);
        assert_eq!(finalized.disks[0].path, "/runtime/overlay.qcow2");
        assert_eq!(finalized.cpus.boot_vcpus, 2);
        assert_eq!(finalized.cpus.max_vcpus, 2);

        let net = finalized.net.expect("net should be Some after finalise");
        assert_eq!(net.len(), 1);
        assert_eq!(net[0].tap, "tap0");

        assert_eq!(finalized.console.mode, "Off");
        assert!(finalized.console.file.is_none());

        assert_eq!(finalized.serial.mode, "File");
        assert_eq!(finalized.serial.file, Some("/var/log/serial.log"));

        // Cmdline now carries the procurator.* tokens too — the network
        // contract is part of `finalize_for_runtime`.
        assert!(
            finalized
                .payload
                .cmdline
                .ends_with(" procurator.ip=10.0.0.2 procurator.gw=10.0.0.1 procurator.pfx=8")
        );
    }

    #[test]
    fn try_from_capnp_builds_create_vm_spec() {
        use capnp::message::Builder;

        let mut builder = Builder::new_default();
        {
            let vm_spec =
                builder.init_root::<commands::common_capnp::vm_spec::Builder<
                    '_,
                    commands::ch_capnp::vm_config::Owned,
                >>();
            let mut spec = vm_spec.init_spec();

            let mut cpus = spec.reborrow().init_cpus();
            cpus.set_boot_vcpus(4);
            cpus.set_max_vcpus(4);

            let mut memory = spec.reborrow().init_memory();
            memory.set_size(1024 * 1024 * 1024);

            let mut payload = spec.reborrow().init_payload();
            payload.set_kernel("/boot/vmlinux");
            payload.set_cmdline("console=ttyS0 root=/dev/vda init=/nix/store/fake/init");
            payload.set_initramfs("/boot/initramfs.img");

            let mut disks = spec.reborrow().init_disks(1);
            disks.reborrow().get(0).set_path("/images/rootfs.raw");

            let mut console = spec.reborrow().init_console();
            console.set_mode("Off");
            console.set_file("");

            let mut serial = spec.reborrow().init_serial();
            serial.set_mode("Tty");
            serial.set_file("");
        }

        let reader = builder
            .get_root_as_reader::<commands::common_capnp::vm_spec::Reader<'_, commands::ch_capnp::vm_config::Owned>>()
            .expect("failed to get reader");

        let spec = CreateVmSpecRef::try_from(reader).expect("try_from should succeed");

        assert_eq!(spec.kernel(), "/boot/vmlinux");
        assert_eq!(spec.initramfs(), "/boot/initramfs.img");
        assert_eq!(spec.root_disk(), "/images/rootfs.raw");

        let vm = spec.vm_config();
        assert_eq!(vm.cpus.boot_vcpus, 4);
        assert_eq!(vm.cpus.max_vcpus, 4);
        assert_eq!(vm.memory.size, 1024 * 1024 * 1024);
        // `cmdline` is the base cmdline read from capnp; the worker appends
        // runtime tokens via `apply_runtime_network` before POSTing to CH.
        assert_eq!(
            vm.payload.cmdline,
            "console=ttyS0 root=/dev/vda init=/nix/store/fake/init"
        );
        assert!(vm.net.is_none());
    }

    #[test]
    fn netmask_to_prefix_common_masks() {
        use std::net::Ipv4Addr;
        assert_eq!(super::netmask_to_prefix(Ipv4Addr::BROADCAST), 32);
        assert_eq!(super::netmask_to_prefix(Ipv4Addr::new(255, 255, 0, 0)), 16);
        assert_eq!(
            super::netmask_to_prefix(Ipv4Addr::new(255, 255, 255, 0)),
            24
        );
        assert_eq!(super::netmask_to_prefix(Ipv4Addr::UNSPECIFIED), 0);
    }

    #[test]
    fn finalize_preserves_base_cmdline_and_appends_procurator() {
        use std::net::Ipv4Addr;
        let base = "console=ttyS0 root=/dev/vda rw init=/nix/store/abcd-nixos-system/init";
        let mut config = make_vm_config("/disk.raw", None);
        config.payload.cmdline = base.to_string();

        let finalized = config.finalize_for_runtime(
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(255, 0, 0, 0),
            "/runtime/disk.img",
            "/runtime/serial.log",
            NetConfigRef::new("tap0", "02:00:00:00:00:01"),
        );

        let out = &finalized.payload.cmdline;
        // Base is preserved verbatim (Nix owns what goes in it); runtime
        // tokens are appended at the end.
        assert!(out.starts_with(base));
        // Three independent tokens — must match the contract enforced by
        // the in-image `procurator-netcfg` unit.
        assert!(out.contains(" procurator.ip=10.0.0.2 "));
        assert!(out.contains(" procurator.gw=10.0.0.1 "));
        assert!(out.ends_with(" procurator.pfx=8"));
        // No stock `ip=` token (we use `procurator.ip=` instead).
        assert!(!out.split_whitespace().any(|t: &str| t.starts_with("ip=")));
    }

    /// Regression guard for the bug we hit on 2026-04-20: emitting
    /// `procurator.ip=<ip>/<pfx>` (CIDR) instead of three independent
    /// tokens makes the in-image `procurator-netcfg` unit fail because it
    /// looks for `procurator.pfx=` separately. The VM then comes up with
    /// no IP and the worker reports `status=running` while ARP fails on
    /// the bridge — a particularly confusing failure mode.
    ///
    /// Locking the exact shape of the produced cmdline keeps the worker
    /// in lock-step with the Nix-side contract in `nix/lib/diskVm.nix`.
    #[test]
    fn finalize_emits_three_separate_tokens_no_cidr() {
        use std::net::Ipv4Addr;
        let mut config = make_vm_config("/disk.raw", None);
        config.payload.cmdline = "BASE".to_string();

        let finalized = config.finalize_for_runtime(
            Ipv4Addr::new(10, 0, 0, 11),
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(255, 0, 0, 0),
            "/runtime/disk.img",
            "/runtime/serial.log",
            NetConfigRef::new("tap0", "02:00:00:00:00:01"),
        );

        let out = &finalized.payload.cmdline;

        // Expected exact shape — matches the dev launcher in
        // `nix/examples/python-workload/flake.nix` byte-for-byte after the
        // base cmdline.
        assert_eq!(
            out,
            "BASE procurator.ip=10.0.0.11 procurator.gw=10.0.0.1 procurator.pfx=8"
        );

        // The IP token must NOT include a `/<prefix>` suffix.
        let ip_token = out
            .split_whitespace()
            .find(|t| t.starts_with("procurator.ip="))
            .expect("procurator.ip token must be present");
        assert!(
            !ip_token.contains('/'),
            "procurator.ip must not be CIDR; got {ip_token:?}"
        );

        // All three tokens must be present and parseable independently.
        let tokens: std::collections::HashMap<&str, &str> = out
            .split_whitespace()
            .filter_map(|t| t.split_once('='))
            .collect();
        assert_eq!(tokens.get("procurator.ip"), Some(&"10.0.0.11"));
        assert_eq!(tokens.get("procurator.gw"), Some(&"10.0.0.1"));
        assert_eq!(tokens.get("procurator.pfx"), Some(&"8"));
    }
}
