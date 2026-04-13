use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PayloadConfigRef<'a> {
    kernel: &'a str,
    cmdline: &'a str,
    initramfs: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
pub struct BackendConfigRef<'a> {
    boot_vcpus: u32,
    max_vcpus: u32,
    memory_size: u64,
    payload: PayloadConfigRef<'a>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVmSpecRef<'a> {
    toplevel: &'a str,
    kernel_path: &'a str,
    initrd_path: &'a str,
    disk_image_path: &'a str,
    cmdline: &'a str,
    network_allowed_domains: Vec<&'a str>,
    backend_config: BackendConfigRef<'a>,
}

impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>>
    for CreateVmSpecRef<'a>
{
    type Error = capnp::Error;

    fn try_from(
        reader: commands::common_capnp::vm_spec::Reader<'a, commands::ch_capnp::vm_config::Owned>,
    ) -> Result<Self, Self::Error> {
        let allowed_domains = reader.get_network_allowed_domains()?;
        let mut network_allowed_domains = Vec::with_capacity(allowed_domains.len() as usize);
        for i in 0..allowed_domains.len() {
            network_allowed_domains.push(allowed_domains.get(i)?.to_str()?);
        }

        let backend = reader.get_backend_config()?;
        let cpus = backend.get_cpus()?;
        let memory = backend.get_memory()?;
        let payload = backend.get_payload()?;

        Ok(Self {
            toplevel: reader.get_toplevel()?.to_str()?,
            kernel_path: reader.get_kernel_path()?.to_str()?,
            initrd_path: reader.get_initrd_path()?.to_str()?,
            disk_image_path: reader.get_disk_image_path()?.to_str()?,
            cmdline: reader.get_cmdline()?.to_str()?,
            network_allowed_domains,
            backend_config: BackendConfigRef {
                boot_vcpus: cpus.get_boot_vcpus(),
                max_vcpus: cpus.get_max_vcpus(),
                memory_size: memory.get_size(),
                payload: PayloadConfigRef {
                    kernel: payload.get_kernel()?.to_str()?,
                    cmdline: payload.get_cmdline()?.to_str()?,
                    initramfs: payload.get_initramfs()?.to_str()?,
                },
            },
        })
    }
}
