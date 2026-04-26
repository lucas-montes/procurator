@0xbdcd86e44f3bb4f1;

# Cloud Hypervisor VmConfig — trimmed to the fields actually used for direct-kernel boot.
struct VmConfig {
	payload @0 :PayloadConfig;
	cpus @1 :CpusConfig;
	memory @2 :MemoryConfig;
	# At least one disk required in practice
	disks @3 :List(DiskConfig);
	# Optional: omit for no-network boot
	net @4 :List(NetConfig);
	# Optional: console output mode (Off | Pty | Tty | File | Socket | Null)
	console @5 :ConsoleConfig;
	# Optional: serial output mode (Off | Pty | Tty | File | Socket | Null)
	serial @6 :ConsoleConfig;
}

# Both fields are required by the Cloud Hypervisor API.
struct CpusConfig {
	bootVcpus @0 :UInt32;
	maxVcpus @1 :UInt32;
}

# size in bytes; required by CH.
struct MemoryConfig {
	size @0 :UInt64;
}

# Direct-kernel boot fields. firmware is not used.
struct PayloadConfig {
	# Path to kernel image (bzImage)
	kernel @0 :Text;
	# Kernel command line
	cmdline @1 :Text;
	# Path to initramfs/initrd
	initramfs @2 :Text;
}

# `path` is the only field the control plane fills in; the worker overwrites
# it at runtime with the per-VM writable copy under `<runtime_dir>/<vm_id>/`.
#
# `imageType` selects the disk format probed by Cloud Hypervisor. Allowed
# values: "raw" | "qcow" | "fixedVhd" | "vhdx" (CH's `DiskConfig::image_type`).
# When empty, CH falls back to magic-byte probing — which has been observed
# to misidentify NixOS raw images as qcow on recent CH releases. The worker
# therefore defaults this to "raw" if the field arrives empty.
struct DiskConfig {
	path @0 :Text;
	imageType @1 :Text;
}

# Only tap is used; CH fills in defaults for ip/mask/mac.
struct NetConfig {
	tap @0 :Text;
	ip @1 :Text;
	mask @2 :Text;
	mac @3 :Text;
}

# mode is required; file is only meaningful when mode = "File".
struct ConsoleConfig {
	mode @0 :Text;
	file @1 :Text;
}
