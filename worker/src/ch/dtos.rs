use std::ops::Not;

use serde::Serialize;


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

/// Maps to `DiskConfig` in ch.capnp. The worker overrides path at runtime.
#[derive(Debug, Clone, Serialize)]
pub struct DiskConfigRef<'a> {
    path: &'a str,
}

/// Maps to `NetConfig` in ch.capnp. Only `tap` is used; CH fills in defaults for the rest.
#[derive(Debug, Clone, Serialize)]
pub struct NetConfigRef<'a> {
    tap: &'a str,
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
    pub fn finalize_for_runtime(
        mut self,
        writable_disk_path: &'a str,
        serial_log_path: &'a str,
        tap_name: Option<&'a str>,
    ) -> Self {
        if let Some(first_disk) = self.disks.first_mut() {
            first_disk.path = writable_disk_path;
        } else {
            self.disks.push(DiskConfigRef {
                path: writable_disk_path,
            });
        }

        self.net = tap_name.map(|name| vec![NetConfigRef { tap: name }]);

        self.console = ConsoleConfigRef {
            mode: "Off",
            file: None,
        };
        self.serial = ConsoleConfigRef {
            mode: "File",
            file: Some(serial_log_path),
        };

        self
    }
}

// ── Capnp reader → Rust struct DTOs ──────────────────────────────────────────
// These types deserialise from capnp messages coming from the control plane.

#[derive(Debug, Clone, Serialize)]
pub struct PayloadConfigRef<'a> {
    kernel: &'a str,
    cmdline: &'a str,
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
            disks.push(DiskConfigRef {
                path: require_non_empty(disk.get_path()?.to_str()?, "disk.path")?,
            });
        }

        let net = match reader.get_net() {
            Ok(net_reader) if net_reader.is_empty().not() => {
                let mut nets = Vec::with_capacity(net_reader.len() as usize);
                for net_cfg in net_reader {
                    nets.push(NetConfigRef {
                        tap: require_non_empty(net_cfg.get_tap()?.to_str()?, "net.tap")?,
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
                    cmdline: require_non_empty(
                        payload.get_cmdline()?.to_str()?,
                        "payload.cmdline",
                    )?,
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
                cmdline: "console=ttyS0",
                initramfs: "/path/to/initramfs",
            },
            disks: vec![DiskConfigRef { path: disk_path }],
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
        let config = make_vm_config("/original/disk.raw", None);

        let finalized = config.finalize_for_runtime(
            "/runtime/overlay.qcow2",
            "/var/log/serial.log",
            Some("tap0"),
        );

        assert_eq!(finalized.disks.len(), 1);
        assert_eq!(finalized.disks[0].path, "/runtime/overlay.qcow2");
        assert_eq!(finalized.cpus.boot_vcpus, 2);
        assert_eq!(finalized.cpus.max_vcpus, 2);

        let net = finalized
            .net
            .expect("net should be Some when network_enabled=true");
        assert_eq!(net.len(), 1);
        assert_eq!(net[0].tap, "tap0");

        assert_eq!(finalized.console.mode, "Off");
        assert!(finalized.console.file.is_none());

        assert_eq!(finalized.serial.mode, "File");
        assert_eq!(finalized.serial.file, Some("/var/log/serial.log"));
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
            payload.set_cmdline("console=hvc0 root=/dev/vda1");
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
        assert_eq!(vm.payload.cmdline, "console=hvc0 root=/dev/vda1");
        assert!(vm.net.is_none());
    }
}
