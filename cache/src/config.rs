//! Runtime configuration loaded from one TOML file.
//! Why: a single file is the source of truth for local runs and `NixOS`.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::domain::cache::{CacheName, CacheSpec, Visibility};
use crate::domain::digest::Digest;
use crate::domain::nix::SigningKey;

/// Why the config was refused.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Read(String),
    /// The TOML is malformed or has unknown keys.
    Parse(String),
    /// A value is present but unacceptable.
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(m) => write!(f, "config file: {m}"),
            Self::Parse(m) => write!(f, "config parse: {m}"),
            Self::Invalid(m) => write!(f, "config invalid: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Validated settings for the `cache` binary.
#[derive(Debug)]
pub struct Config {
    addr: SocketAddr,
    storage_dir: PathBuf,
    admin_token: Option<Digest>,
    gc_interval: Duration,
    orphan_grace_secs: i64,
    caches: Vec<CacheSpec>,
}

/// The file's shape, before validation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default = "default_addr")]
    addr: String,
    #[serde(default = "default_storage_dir")]
    storage_dir: PathBuf,
    /// Hex SHA-256 of the bootstrap admin token.
    #[serde(default)]
    admin_token_sha256: Option<String>,
    #[serde(default)]
    gc: GcSection,
    #[serde(default)]
    caches: BTreeMap<String, CacheSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GcSection {
    #[serde(default = "default_gc_interval")]
    interval_secs: u64,
    #[serde(default = "default_orphan_grace")]
    orphan_grace_secs: u32,
}

impl Default for GcSection {
    fn default() -> Self {
        Self {
            interval_secs: default_gc_interval(),
            orphan_grace_secs: default_orphan_grace(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheSection {
    #[serde(default)]
    kind: Kind,
    visibility: VisibilityName,
    sign_key_file: PathBuf,
    #[serde(default = "default_store_dir")]
    store_dir: String,
    #[serde(default = "default_priority")]
    priority: u32,
    /// Quota such as `50G`; `0` is unlimited.
    #[serde(default = "default_max_size")]
    max_size: String,
    /// Days an unread path is kept; `0` keeps it until space runs out.
    #[serde(default)]
    retention_days: u32,
}

/// Which front-end a cache speaks. Why: Bazel and REAPI come later.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    #[default]
    Nix,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VisibilityName {
    Public,
    Private,
}

fn default_addr() -> String {
    String::from("127.0.0.1:8081")
}

fn default_storage_dir() -> PathBuf {
    PathBuf::from("/var/lib/procurator-cache")
}

fn default_gc_interval() -> u64 {
    3600
}

fn default_orphan_grace() -> u32 {
    86_400
}

fn default_store_dir() -> String {
    String::from("/nix/store")
}

fn default_priority() -> u32 {
    30
}

fn default_max_size() -> String {
    String::from("0")
}

/// Parse `50G` / `100M` / `10K` / plain bytes.
fn parse_size(raw: &str) -> Option<u64> {
    let text = raw.trim();
    let (number, unit) = match text.char_indices().next_back()? {
        (at, 'G' | 'g') => (&text[..at], 1 << 30),
        (at, 'M' | 'm') => (&text[..at], 1 << 20),
        (at, 'K' | 'k') => (&text[..at], 1 << 10),
        _ => (text, 1_u64),
    };
    number.trim().parse::<u64>().ok()?.checked_mul(unit)
}

/// Build one cache from its config table.
fn cache_spec(name: &str, section: CacheSection) -> Result<CacheSpec, ConfigError> {
    let invalid = |what: fmt::Arguments<'_>| ConfigError::Invalid(format!("caches.{name}: {what}"));
    let Kind::Nix = section.kind;
    let cache_name = CacheName::parse(name).map_err(|e| invalid(format_args!("{e}")))?;
    let key_text = std::fs::read_to_string(&section.sign_key_file).map_err(|e| {
        invalid(format_args!(
            "sign_key_file {}: {e}",
            section.sign_key_file.display()
        ))
    })?;
    let key = SigningKey::parse(&key_text).map_err(|e| invalid(format_args!("{e}")))?;
    let quota = parse_size(&section.max_size)
        .ok_or_else(|| invalid(format_args!("bad max_size {:?}", section.max_size)))?;
    if !section.store_dir.starts_with('/') || section.store_dir.ends_with('/') {
        return Err(invalid(format_args!(
            "store_dir must be absolute with no trailing slash"
        )));
    }
    let retention =
        (section.retention_days > 0).then(|| i64::from(section.retention_days) * 86_400);
    let visibility = match section.visibility {
        VisibilityName::Public => Visibility::Public,
        VisibilityName::Private => Visibility::Private,
    };
    Ok(CacheSpec::new(
        cache_name,
        visibility,
        section.store_dir,
        section.priority,
        quota,
        retention,
        key,
    ))
}

impl Config {
    /// Load and validate a config file.
    ///
    /// # Errors
    /// Returns [`ConfigError`] when the file is unreadable, malformed, names
    /// no cache, or holds an invalid value. Fail-fast: nothing starts on error.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Read(format!("{}: {e}", path.display())))?;
        let file: FileConfig =
            toml::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let addr = file
            .addr
            .parse()
            .map_err(|_| ConfigError::Invalid(format!("addr {:?} is not ip:port", file.addr)))?;
        let admin_token = file
            .admin_token_sha256
            .as_deref()
            .map(|hex| {
                Digest::from_hex(hex).map_err(|_| {
                    ConfigError::Invalid(String::from("admin_token_sha256 must be 64 hex chars"))
                })
            })
            .transpose()?;
        if file.caches.is_empty() {
            return Err(ConfigError::Invalid(String::from(
                "define at least one [caches.<name>]",
            )));
        }
        if file.gc.interval_secs == 0 {
            return Err(ConfigError::Invalid(String::from(
                "gc.interval_secs must be positive",
            )));
        }
        let caches = file
            .caches
            .into_iter()
            .map(|(name, section)| cache_spec(&name, section))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            addr,
            storage_dir: file.storage_dir,
            admin_token,
            gc_interval: Duration::from_secs(file.gc.interval_secs),
            orphan_grace_secs: i64::from(file.gc.orphan_grace_secs),
            caches,
        })
    }

    /// Address to listen on.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Directory holding blobs and the index.
    #[must_use]
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    /// Hash of the bootstrap admin token, if configured.
    #[must_use]
    pub fn admin_token(&self) -> Option<Digest> {
        self.admin_token
    }

    /// How often GC runs.
    #[must_use]
    pub fn gc_interval(&self) -> Duration {
        self.gc_interval
    }

    /// How long an uploaded NAR may wait for its narinfo.
    #[must_use]
    pub fn orphan_grace_secs(&self) -> i64 {
        self.orphan_grace_secs
    }

    /// Take the configured caches.
    #[must_use]
    pub fn into_caches(self) -> Vec<CacheSpec> {
        self.caches
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Config, ConfigError, parse_size};
    use crate::domain::cache::Visibility;

    /// Write a config plus a signing key into a temp dir and load it.
    fn load(body: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        // 32 zero bytes, base64.
        std::fs::write(
            &key,
            "test-1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n",
        )
        .unwrap();
        let path = dir.path().join("cache.toml");
        std::fs::write(&path, body.replace("KEY", &key.display().to_string())).unwrap();
        Config::from_file(&path)
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("0"), Some(0));
        assert_eq!(parse_size("50G"), Some(50 << 30));
        assert_eq!(parse_size("100m"), Some(100 << 20));
        assert_eq!(parse_size("10K"), Some(10 << 10));
        assert_eq!(parse_size("123"), Some(123));
        assert_eq!(parse_size("bogus"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("99999999999999999999G"), None);
    }

    #[test]
    fn loads_public_and_private_caches() {
        let config = load(
            r#"
addr = "127.0.0.1:9000"
storage_dir = "/tmp/x"
admin_token_sha256 = "1734d503f6aa6a047c36d113cbad769f719c93784b469b771c4c3e7c63adbefd"
[gc]
interval_secs = 60
[caches.public]
visibility = "public"
sign_key_file = "KEY"
[caches.internal]
kind = "nix"
visibility = "private"
sign_key_file = "KEY"
max_size = "10G"
retention_days = 30
priority = 10
"#,
        )
        .unwrap();
        assert_eq!(config.addr().port(), 9000);
        assert_eq!(config.storage_dir(), Path::new("/tmp/x"));
        assert!(config.admin_token().is_some());
        assert_eq!(config.gc_interval().as_secs(), 60);
        assert_eq!(config.orphan_grace_secs(), 86_400);
        let caches = config.into_caches();
        // Why: tables load in name order.
        assert_eq!(caches[0].name().as_str(), "internal");
        assert_eq!(caches[0].visibility(), Visibility::Private);
        assert_eq!(caches[0].quota_bytes(), 10 << 30);
        assert_eq!(caches[0].retention_secs(), Some(30 * 86_400));
        assert_eq!(caches[0].priority(), 10);
        assert_eq!(caches[1].visibility(), Visibility::Public);
        assert_eq!(caches[1].store_dir(), "/nix/store");
        assert_eq!(caches[1].quota_bytes(), 0);
        assert_eq!(caches[1].retention_secs(), None);
        assert_eq!(caches[1].signing_key().name(), "test-1");
    }

    #[test]
    fn refuses_bad_configs() {
        let cache = "[caches.c]\nvisibility = \"public\"\nsign_key_file = \"KEY\"\n";
        let cases = [
            (String::from("addr = \"127.0.0.1:1\"\n"), "at least one"),
            (format!("addr = \"nope\"\n{cache}"), "not ip:port"),
            (format!("admin_token_sha256 = \"abc\"\n{cache}"), "64 hex"),
            (format!("{cache}max_size = \"lots\"\n"), "bad max_size"),
            (format!("{cache}store_dir = \"nix/store\"\n"), "store_dir"),
            (format!("{cache}unknown = 1\n"), "unknown field"),
            (
                format!("[gc]\ninterval_secs = 0\n{cache}"),
                "must be positive",
            ),
            (cache.replace("caches.c", "caches.Bad_Name"), "cache names"),
            (cache.replace("KEY", "/nonexistent/key"), "sign_key_file"),
            (cache.replace("public", "open"), "unknown variant"),
            (
                String::from("[caches.c]\nvisibility = \"public\"\n"),
                "sign_key_file",
            ),
        ];
        for (body, expected) in cases {
            let error = load(&body).unwrap_err().to_string();
            assert!(error.contains(expected), "{body:?} gave {error:?}");
        }
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let error = Config::from_file(Path::new("/nonexistent-cache.toml")).unwrap_err();
        assert!(matches!(error, ConfigError::Read(_)));
    }
}
