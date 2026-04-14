@0xbdcd86e44f3bb4f1;

# Cloud Hypervisor VmConfig — fields mirror the CH OpenAPI schema.
# Required fields in OpenAPI are non-optional here.
# Optional OpenAPI fields are represented as Text (empty = absent) or UInt64 = 0 meaning absent.
struct VmConfig {
	# OpenAPI: required
	payload @0 :PayloadConfig;
	# OpenAPI: optional
	cpus @1 :CpusConfig;
	# OpenAPI: optional
	memory @2 :MemoryConfig;
	# OpenAPI: optional — at least one disk expected in practice
	disks @3 :List(DiskConfig);
	# OpenAPI: optional
	net @4 :List(NetConfig);
	# OpenAPI: optional — uses same ConsoleConfig schema as serial
	console @5 :ConsoleConfig;
	# OpenAPI: optional — uses same ConsoleConfig schema as console
	serial @6 :ConsoleConfig;
	# OpenAPI: optional
	rng @7 :RngConfig;
}

# CpusConfig — boot_vcpus and max_vcpus are both required by OpenAPI (minimum: 1).
struct CpusConfig {
	bootVcpus @0 :UInt32;
	maxVcpus @1 :UInt32;
}

# MemoryConfig — size (bytes) is required by OpenAPI.
struct MemoryConfig {
	size @0 :UInt64;
}

# PayloadConfig — all fields are optional in OpenAPI; kernel is used for direct-kernel boot.
struct PayloadConfig {
	# Optional: path to firmware (OVMF/EDK2); mutually exclusive with kernel
	firmware @0 :Text;
	# Optional: path to kernel image (bzImage)
	kernel @1 :Text;
	# Optional: kernel command line
	cmdline @2 :Text;
	# Optional: path to initramfs/initrd
	initramfs @3 :Text;
}

# DiskConfig — path is the only meaningful required field for VM creation.
struct DiskConfig {
	path @0 :Text;
	# Optional: default false
	readonly @1 :Bool;
	# Optional: default false — enables O_DIRECT
	direct @2 :Bool;
}

# NetConfig — all fields are optional in OpenAPI; tap is the common field we use.
struct NetConfig {
	# Optional: tap device name
	tap @0 :Text;
	# Optional: guest-facing IPv4/IPv6 address (default "192.168.249.1")
	ip @1 :Text;
	# Optional: netmask (default "255.255.255.0")
	mask @2 :Text;
	# Optional: guest MAC address
	mac @3 :Text;
	# Optional: host-side MAC address
	hostMac @4 :Text;
	# Optional: MTU override
	mtu @5 :UInt32;
}

# RngConfig — src is required by OpenAPI.
struct RngConfig {
	src @0 :Text;
}

# ConsoleConfig — mode is required; file is only used when mode = "File".
# Used for both `console` and `serial` fields in VmConfig.
struct ConsoleConfig {
	# Required: one of Off | Pty | Tty | File | Socket | Null
	mode @0 :Text;
	# Optional: output file path; only meaningful when mode = "File"
	file @1 :Text;
}
