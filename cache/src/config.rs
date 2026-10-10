//! Runtime configuration loaded from a TOML file.
//! Why: one file is the single source of truth for local runs and NixOS.

use std::path::{Path, PathBuf};

/// All knobs for the cache. See `nix/modules/cache.nix` for NixOS wiring.
/// Built from [`FileConfig`] so IO and validation stay out of deserialization.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bind address, e.g. `0.0.0.0:8081`.
    pub addr: String,
    /// Nix store directory proxied on read miss.
    pub store_dir: String,
    /// Persistent directory for uploaded `*.narinfo` + `nar/*`.
    pub storage_dir: PathBuf,
    /// Soft quota in bytes (0 = unlimited). Why: prevents disk exhaustion.
    pub max_size_bytes: u64,
    /// Advertised priority in `/nix-cache-info`.
    pub priority: u32,
    /// Cache signing keys, loaded from `sign_key_files`.
    pub sign_keys: Vec<SignKey>,
    /// TLS settings.
    pub tls: TlsSection,
    /// Uniform access switches (same options on every deployment).
    pub access: AccessSection,
}

/// TLS settings (`[tls]` table; secrets stay as paths, never inline).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSection {
    /// Which TLS provider to build (`file` only for now; vault/step/spire later).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Server certificate chain (PEM). Absent = plain HTTP (dev only).
    #[serde(default)]
    pub cert_file: Option<PathBuf>,
    /// Server private key (PEM).
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    /// Client CA bundle (PEM). Set = require mTLS client certs.
    #[serde(default)]
    pub client_ca_file: Option<PathBuf>,
    /// Allowed writer identities (`CN` or `sha256:<hex fp>`), empty = any verified client.
    #[serde(default)]
    pub writer_allowlist: Vec<String>,
}

/// Uniform access switches (`[access]` table; identical on every deployment).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessSection {
    /// Require a verified client even for `GET`/`HEAD`. Why: off keeps substituters usable.
    #[serde(default)]
    pub reads_require_auth: bool,
    /// Write policy; `auto` = open on loopback bind, required otherwise.
    #[serde(default)]
    pub writes_require_auth: WritesAuth,
    /// Treat `127.0.0.1`/`::1` as trusted without a cert. Why: co-located CI needs zero creds.
    #[serde(default = "default_true")]
    pub loopback_bypass: bool,
}

/// Write policy with a bind-aware default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WritesAuth {
    /// Open on loopback bind, required otherwise.
    #[default]
    Auto,
    /// Always require a verified client (or loopback bypass).
    Always,
    /// Never require (refused at startup on non-loopback binds).
    Never,
}

/// One ed25519 cache signing key.
#[derive(Debug, Clone)]
pub struct SignKey {
    /// Key name, e.g. `cache.local-1`.
    pub name: String,
    /// Raw 32-byte seed.
    pub seed: [u8; 32],
}

/// Concrete config errors. Why: no `anyhow`, callers match on variants.
#[derive(Debug)]
pub enum ConfigError {
    /// Config file missing or unreadable.
    NotFound(String),
    /// TOML parse or value error (includes line info from the parser).
    Parse(String),
    /// A signing key file was missing or malformed.
    SignKey(String),
    /// Refused to start: open writes on a public bind.
    Insecure(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "config file: {m}"),
            Self::Parse(m) => write!(f, "config parse: {m}"),
            Self::SignKey(m) => write!(f, "bad signing key: {m}"),
            Self::Insecure(m) => write!(f, "refusing insecure config: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Raw file shape. Why: keeps IO/validation out of `Deserialize`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    /// Bind address.
    #[serde(default = "default_addr")]
    addr: String,
    /// Nix store directory proxied on read miss.
    #[serde(default = "default_store_dir")]
    store_dir: String,
    /// Persistent directory for uploaded objects.
    #[serde(default = "default_storage_dir")]
    storage_dir: PathBuf,
    /// Soft quota (`50G`, `100M`, `0` = unlimited).
    #[serde(
        default = "default_max_size",
        rename = "max_size",
        deserialize_with = "de_size"
    )]
    max_size_bytes: u64,
    /// Advertised priority.
    #[serde(default = "default_priority")]
    priority: u32,
    /// Signing key files (`name:base64` lines, 32 or 64 bytes).
    #[serde(default)]
    sign_key_files: Vec<PathBuf>,
    /// TLS settings.
    #[serde(default)]
    tls: TlsSection,
    /// Access switches.
    #[serde(default)]
    access: AccessSection,
}

impl Default for TlsSection {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            cert_file: None,
            key_file: None,
            client_ca_file: None,
            writer_allowlist: Vec::new(),
        }
    }
}

impl Default for AccessSection {
    fn default() -> Self {
        Self {
            reads_require_auth: false,
            writes_require_auth: WritesAuth::Auto,
            loopback_bypass: true,
        }
    }
}

fn default_addr() -> String {
    "0.0.0.0:8081".to_string()
}

fn default_store_dir() -> String {
    "/nix/store".to_string()
}

fn default_storage_dir() -> PathBuf {
    PathBuf::from("/var/lib/procurator-cache")
}

fn default_max_size() -> u64 {
    50 * 1_073_741_824
}

fn default_priority() -> u32 {
    30
}

fn default_provider() -> String {
    "file".to_string()
}

fn default_true() -> bool {
    true
}

/// Deserialize `max_size` (`"50G"`, `"0"`) straight to bytes.
fn de_size<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let raw = String::deserialize(d)?;
    parse_size(&raw).map_err(serde::de::Error::custom)
}

impl Config {
    /// Load and validate `--config <path>`. Why: file-only, no env fallback.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::NotFound(format!("{}: {e}", path.display())))?;
        let fc: FileConfig =
            toml::from_str(&raw).map_err(|e| ConfigError::Parse(format!("{e}")))?;
        Self::load(fc)
    }

    /// Resolve key files and enforce the fail-closed rule.
    fn load(fc: FileConfig) -> Result<Self, ConfigError> {
        let sign_keys = load_sign_keys(&fc.sign_key_files)?;
        let cfg = Self {
            addr: fc.addr,
            store_dir: fc.store_dir,
            storage_dir: fc.storage_dir,
            max_size_bytes: fc.max_size_bytes,
            priority: fc.priority,
            sign_keys,
            tls: fc.tls,
            access: fc.access,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Fail-closed: open writes on a public bind refuse to start.
    fn validate(&self) -> Result<(), ConfigError> {
        // Why: parse first so a typo'd addr can't slip past the bind check.
        let loopback = bind_is_loopback(&self.addr)
            .ok_or_else(|| ConfigError::Parse(format!("bad addr (want ip:port): {}", self.addr)))?;
        if self.writes_open() && !loopback {
            return Err(ConfigError::Insecure(format!(
                "writes are open on public bind {} (set [access] writes_require_auth)",
                self.addr
            )));
        }
        Ok(())
    }

    /// True when PUTs need no client cert (loopback bypass handled per-peer).
    #[must_use]
    pub fn writes_open(&self) -> bool {
        match self.access.writes_require_auth {
            WritesAuth::Never => true,
            WritesAuth::Always => false,
            WritesAuth::Auto => bind_is_loopback(&self.addr).unwrap_or(false),
        }
    }

    /// True when server cert+key are configured.
    #[must_use]
    pub fn tls_enabled(&self) -> bool {
        self.tls.cert_file.is_some() && self.tls.key_file.is_some()
    }

    /// True when client certs are required (mTLS).
    #[must_use]
    pub fn mtls_enabled(&self) -> bool {
        self.tls_enabled() && self.tls.client_ca_file.is_some()
    }
}

/// Parse the bind address; `None` = unparseable (caller fails closed).
fn bind_is_loopback(addr: &str) -> Option<bool> {
    addr.parse::<std::net::SocketAddr>()
        .map(|a| a.ip().is_loopback())
        .ok()
}

/// Parse `50G`/`100M`/`10K`/`0`/bytes into bytes. Why: human-friendly quota.
fn parse_size(raw: &str) -> Result<u64, String> {
    let s = raw.trim();
    if s == "0" {
        return Ok(0);
    }
    let (num, mult) = match s.strip_suffix(['G', 'g']) {
        Some(n) => (n, 1_073_741_824_u64),
        None => match s.strip_suffix(['M', 'm']) {
            Some(n) => (n, 1_048_576_u64),
            None => match s.strip_suffix(['K', 'k']) {
                Some(n) => (n, 1_024_u64),
                None => (s, 1_u64),
            },
        },
    };
    num.trim()
        .parse::<u64>()
        .map(|n| n.saturating_mul(mult))
        .map_err(|_| format!("bad size: {raw}"))
}

/// Load `name:base64` keys from listed files.
/// Why: matches `nix-store --generate-binary-cache-key` output format.
fn load_sign_keys(files: &[PathBuf]) -> Result<Vec<SignKey>, ConfigError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut out = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::SignKey(format!("{}: {e}", path.display())))?;
        for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (name, b64) = line
                .split_once(':')
                .ok_or_else(|| ConfigError::SignKey(format!("{}: missing ':'", path.display())))?;
            let bytes = STANDARD
                .decode(b64.trim())
                .map_err(|e| ConfigError::SignKey(format!("{}: base64: {e}", path.display())))?;
            // Why: nix secret files may hold 64B (seed+pub); first 32B is the seed.
            if bytes.len() != 32 && bytes.len() != 64 {
                return Err(ConfigError::SignKey(format!(
                    "{}: want 32 or 64 bytes, got {}",
                    path.display(),
                    bytes.len()
                )));
            }
            let mut seed = [0_u8; 32];
            seed.copy_from_slice(&bytes[..32]);
            out.push(SignKey {
                name: name.trim().to_string(),
                seed,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Config, parse_size};
    use std::path::PathBuf;

    /// Write a temp file that won't collide across parallel tests.
    fn temp_path(name: &str, body: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pcr-cache-test-{name}-{nanos}.toml"));
        std::fs::write(&path, body).expect("write temp config");
        path
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("50G").unwrap(), 50 * 1_073_741_824);
        assert_eq!(parse_size("100m").unwrap(), 100 * 1_048_576);
        assert_eq!(parse_size("10K").unwrap(), 10 * 1_024);
        assert!(parse_size("bogus").is_err());
    }

    #[test]
    fn minimal_file_uses_defaults() {
        let path = temp_path("minimal", "storage_dir = \"/tmp/x\"\n");
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.addr, "0.0.0.0:8081");
        assert_eq!(cfg.store_dir, "/nix/store");
        assert_eq!(cfg.max_size_bytes, 50 * 1_073_741_824);
        assert_eq!(cfg.priority, 30);
        assert!(!cfg.access.reads_require_auth);
        assert!(cfg.access.loopback_bypass);
        assert!(!cfg.tls_enabled());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn full_file_parses_sections() {
        let body = r#"
addr = "127.0.0.1:8081"
max_size = "10G"
priority = 40
[tls]
provider = "file"
writer_allowlist = ["ci-writer"]
[access]
reads_require_auth = true
writes_require_auth = "always"
loopback_bypass = false
"#;
        let path = temp_path("full", body);
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.addr, "127.0.0.1:8081");
        assert_eq!(cfg.max_size_bytes, 10 * 1_073_741_824);
        assert_eq!(cfg.priority, 40);
        assert_eq!(cfg.tls.writer_allowlist, ["ci-writer"]);
        assert!(cfg.access.reads_require_auth);
        assert!(!cfg.access.loopback_bypass);
        assert!(!cfg.writes_open());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_open_writes_on_public_bind() {
        let body = "addr = \"0.0.0.0:8081\"\nstorage_dir = \"/tmp/x\"\n\
                    [access]\nwrites_require_auth = \"never\"\n";
        let path = temp_path("insecure", body);
        let err = Config::from_file(&path).unwrap_err().to_string();
        assert!(err.contains("refusing insecure"), "got: {err}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_errors() {
        let err = Config::from_file(PathBuf::from("/nonexistent-cache.toml").as_path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("nonexistent"), "got: {err}");
    }
}
