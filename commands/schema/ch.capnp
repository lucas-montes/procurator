@0xbdcd86e44f3bb4f1;

struct VmConfig {
	cpus @0 :CpusConfig;
	memory @1 :MemoryConfig;
	payload @2 :PayloadConfig;
	disks @3 :List(DiskConfig);
	net @4 :List(NetConfig);
	rng @5 :RngConfig;
	console @6 :ConsoleConfig;
	serial @7 :SerialConfig;
}



struct CpusConfig {
	bootVcpus @0 :UInt32;
	maxVcpus @1 :UInt32;
}



struct MemoryConfig {
	size @0 :UInt64;
}

struct PayloadConfig {
	kernel @0 :Text;
	cmdline @1 :Text;
	initramfs @2 :Text;
}


struct DiskConfig {
	path @0 :Text;
	readonly @1 :Bool;
	direct @2 :Bool;
}


struct NetConfig {
	tap @0 :Text;
	ip @1 :Text;
	mask @2 :Text;
	mac @3 :Text;
}

struct RngConfig {
	src @0 :Text;
}


enum ConsoleMode {
	off @0;
	pty @1;
	tty @2;
	file @3;
	socket @4;
	null @5;
}

struct ConsoleConfig {
	mode @0 :Text;
}

struct SerialConfig {
	mode @0 :Text;
	file @1 :Text;
}
