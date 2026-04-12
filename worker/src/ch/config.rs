use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub cpus: CpusConfig,
    pub memory: MemoryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub disks: Vec<DiskConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Vec<NetConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rng: Option<RngConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<ConsoleConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<SerialConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpusConfig {
    pub boot_vcpus: u8,
    pub max_vcpus: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadConfig {
    pub kernel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initramfs: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskConfig {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RngConfig {
    pub src: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}
