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

# Only boot_vcpus is sent; CH defaults max to boot when omitted.
struct CpusConfig {
	bootVcpus @0 :UInt32;
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

# Only path is sent; readonly/direct are managed by the worker at runtime.
struct DiskConfig {
	path @0 :Text;
}

# Only tap is used; CH fills in defaults for ip/mask/mac.
struct NetConfig {
	tap @0 :Text;
}

# mode is required; file is only meaningful when mode = "File".
struct ConsoleConfig {
	mode @0 :Text;
	file @1 :Text;
}
