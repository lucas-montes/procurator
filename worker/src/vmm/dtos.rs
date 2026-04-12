impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a>> for VmSpecRef<'a> {
    type Error = capnp::Error;

    fn try_from(r: commands::common_capnp::vm_spec::Reader<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            toplevel: r.get_toplevel()?.to_str()?,
            kernel_path: r.get_kernel_path()?.to_str()?,
            initrd_path: r.get_initrd_path()?.to_str()?,
            disk_image_path: r.get_disk_image_path()?.to_str()?,
            cmdline: r.get_cmdline()?.to_str()?,
            cpu: r.get_cpu(),
            memory_mb: r.get_memory_mb(),
        })
    }
}

#[derive(Debug)]
pub struct VmSpecRef<'a> {
    toplevel: &'a str,
    kernel_path: &'a str,
    initrd_path: &'a str,
    disk_image_path: &'a str,
    cmdline: &'a str,
    cpu: u32,
    memory_mb: u32,
}
