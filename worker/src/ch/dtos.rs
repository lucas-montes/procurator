use serde::Serialize;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    console: Option<ConsoleConfigRef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<ConsoleConfigRef<'a>>,
}

/// Maps to `CpusConfig` in ch.capnp.
#[derive(Debug, Clone, Serialize)]
pub struct CpusConfigRef {
    boot_vcpus: u32,
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
    mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
}

impl<'a> VmConfigRef<'a> {
    pub fn finalize_for_runtime(
        mut self,
        writable_disk_path: &'a str,
        serial_log_path: &'a str,
        tap_name: &'a str,
        network_enabled: bool,
    ) -> Self {
        if let Some(first_disk) = self.disks.first_mut() {
            first_disk.path = writable_disk_path;
        } else {
            self.disks.push(DiskConfigRef {
                path: writable_disk_path,
            });
        }

        self.net = if network_enabled {
            Some(vec![NetConfigRef { tap: tap_name }])
        } else {
            None
        };

        self.console = Some(ConsoleConfigRef {
            mode: "Off",
            file: None,
        });
        self.serial = Some(ConsoleConfigRef {
            mode: "File",
            file: Some(serial_log_path),
        });

        self
    }
}

// ── Capnp reader → Rust struct DTOs ──────────────────────────────────────────
// These types deserialise from capnp messages coming from the control plane.

#[derive(Debug, Clone, Serialize)]
pub struct PayloadConfigRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    kernel: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cmdline: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initramfs: Option<&'a str>,
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

impl<'a> TryFrom<commands::ch_capnp::vm_config::Reader<'a>> for VmConfigRef<'a> {
    type Error = capnp::Error;

    fn try_from(reader: commands::ch_capnp::vm_config::Reader<'a>) -> Result<Self, Self::Error> {
        let cpus = reader.get_cpus()?;
        let memory = reader.get_memory()?;
        let payload = reader.get_payload()?;

        let disks_reader = reader.get_disks()?;
        let mut disks = Vec::with_capacity(disks_reader.len() as usize);
        for i in 0..disks_reader.len() {
            let disk = disks_reader.get(i);
            disks.push(DiskConfigRef {
                path: disk.get_path()?.to_str()?,
            });
        }

        let net = match reader.get_net() {
            Ok(net_reader) if net_reader.len() > 0 => {
                let mut nets = Vec::with_capacity(net_reader.len() as usize);
                for i in 0..net_reader.len() {
                    let net_cfg = net_reader.get(i);
                    let tap = net_cfg.get_tap()?.to_str()?;
                    if !tap.is_empty() {
                        nets.push(NetConfigRef { tap });
                    }
                }
                if nets.is_empty() { None } else { Some(nets) }
            }
            _ => None,
        };

        let console = match reader.get_console() {
            Ok(console_reader) => {
                let mode = console_reader.get_mode()?.to_str()?;
                if mode.is_empty() {
                    None
                } else {
                    Some(ConsoleConfigRef {
                        mode,
                        file: non_empty(console_reader.get_file()?.to_str()?),
                    })
                }
            }
            Err(_) => None,
        };

        let serial = match reader.get_serial() {
            Ok(serial_reader) => {
                let mode = serial_reader.get_mode()?.to_str()?;
                if mode.is_empty() {
                    None
                } else {
                    Some(ConsoleConfigRef {
                        mode,
                        file: non_empty(serial_reader.get_file()?.to_str()?),
                    })
                }
            }
            Err(_) => None,
        };

        Ok(Self {
            cpus: CpusConfigRef {
                boot_vcpus: cpus.get_boot_vcpus(),
            },
            memory: MemoryConfigRef {
                size: memory.get_size(),
            },
            payload: PayloadConfigRef {
                kernel: non_empty(payload.get_kernel()?.to_str()?),
                cmdline: non_empty(payload.get_cmdline()?.to_str()?),
                initramfs: non_empty(payload.get_initramfs()?.to_str()?),
            },
            disks,
            net,
            console,
            serial,
        })
    }
}

#[derive(Debug)]
pub struct CreateVmSpecRef<'a> {
    vm_config: VmConfigRef<'a>,
}

impl<'a> CreateVmSpecRef<'a> {
    pub fn vm_config(&self) -> &VmConfigRef<'a> {
        &self.vm_config
    }

    pub fn kernel(&self) -> Option<&str> {
        self.vm_config.payload.kernel
    }

    pub fn initramfs(&self) -> Option<&str> {
        self.vm_config.payload.initramfs
    }

    pub fn root_disk(&self) -> Option<&str> {
        self.vm_config.disks.first().map(|disk| disk.path)
    }
}

impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>>
    for CreateVmSpecRef<'a>
{
    type Error = capnp::Error;

    fn try_from(
        reader: commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>,
    ) -> Result<Self, Self::Error> {
        let vm_config = VmConfigRef::try_from(reader.get_spec()?)?;

        Ok(Self { vm_config })
    }
}
