//! Named caches: the unit of tenancy, visibility, quota, and retention.

use std::fmt;

use super::nix::SigningKey;

/// Longest accepted cache name.
const MAX_NAME_LEN: usize = 63;

/// Names that would shadow service routes.
const RESERVED: [&str; 3] = ["healthz", "readyz", "metrics"];

/// A validated cache name: lowercase letters, digits, and inner dashes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheName(String);

/// A cache name that failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCacheName;

impl fmt::Display for InvalidCacheName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cache names are 1-63 chars of [a-z0-9-], starting with a letter or digit")
    }
}

impl std::error::Error for InvalidCacheName {}

impl CacheName {
    /// Validate a cache name.
    ///
    /// # Errors
    /// Returns [`InvalidCacheName`] for empty, overlong, reserved, or
    /// badly-charactered names.
    pub fn parse(raw: &str) -> Result<Self, InvalidCacheName> {
        let valid_chars = raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let valid_edges = !raw.starts_with('-') && !raw.ends_with('-');
        if raw.is_empty()
            || raw.len() > MAX_NAME_LEN
            || !valid_chars
            || !valid_edges
            || RESERVED.contains(&raw)
        {
            return Err(InvalidCacheName);
        }
        Ok(Self(String::from(raw)))
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who may read a cache without credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Anyone reads; writing needs a grant.
    Public,
    /// Reading and writing both need a grant.
    Private,
}

/// One configured cache.
#[derive(Debug)]
pub struct CacheSpec {
    name: CacheName,
    visibility: Visibility,
    store_dir: String,
    priority: u32,
    quota_bytes: u64,
    retention_secs: Option<i64>,
    signing_key: SigningKey,
}

impl CacheSpec {
    /// Describe a cache. `quota_bytes == 0` means unlimited; `retention_secs
    /// == None` keeps entries until evicted for space.
    #[must_use]
    pub fn new(
        name: CacheName,
        visibility: Visibility,
        store_dir: String,
        priority: u32,
        quota_bytes: u64,
        retention_secs: Option<i64>,
        signing_key: SigningKey,
    ) -> Self {
        Self {
            name,
            visibility,
            store_dir,
            priority,
            quota_bytes,
            retention_secs,
            signing_key,
        }
    }

    /// The cache's name.
    #[must_use]
    pub fn name(&self) -> &CacheName {
        &self.name
    }

    /// Whether anonymous reads are allowed.
    #[must_use]
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// The Nix store prefix this cache serves (normally `/nix/store`).
    #[must_use]
    pub fn store_dir(&self) -> &str {
        &self.store_dir
    }

    /// Substituter priority advertised to clients (lower wins).
    #[must_use]
    pub fn priority(&self) -> u32 {
        self.priority
    }

    /// Byte quota, `0` for unlimited.
    #[must_use]
    pub fn quota_bytes(&self) -> u64 {
        self.quota_bytes
    }

    /// Seconds an unread entry is kept, if bounded.
    #[must_use]
    pub fn retention_secs(&self) -> Option<i64> {
        self.retention_secs
    }

    /// The key this cache signs narinfos with.
    #[must_use]
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }
}

#[cfg(test)]
mod tests {
    use super::CacheName;

    #[test]
    fn accepts_dns_like_names() {
        assert!(CacheName::parse("public").is_ok());
        assert!(CacheName::parse("team-a1").is_ok());
        assert_eq!(CacheName::parse("x").unwrap().as_str(), "x");
    }

    #[test]
    fn rejects_unsafe_or_reserved_names() {
        for bad in [
            "", "-a", "a-", "A", "_api", "a/b", "a.b", "healthz", "metrics",
        ] {
            assert!(CacheName::parse(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(CacheName::parse(&"a".repeat(64)).is_err());
    }
}
