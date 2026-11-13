//! Who is calling and what they may do. Pure decisions, no IO.
//! Why: every interface (HTTP, in-process) authorizes through the same rules.

use std::fmt;

use super::cache::{CacheName, Visibility};
use super::digest::Digest;

/// What a grant allows on one cache. `Write` includes `Read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Fetch objects.
    Read,
    /// Fetch and upload objects.
    Write,
}

impl Permission {
    /// Parse the wire form (`read` | `write`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            _ => None,
        }
    }

    /// The wire form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }

    fn allows(self, action: Action) -> bool {
        matches!(
            (self, action),
            (Self::Write, _) | (Self::Read, Action::Read)
        )
    }
}

/// What a request wants to do to a cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Fetch objects.
    Read,
    /// Upload objects.
    Write,
}

/// One permission on one cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    cache: CacheName,
    permission: Permission,
}

impl Grant {
    /// Grant `permission` on `cache`.
    #[must_use]
    pub fn new(cache: CacheName, permission: Permission) -> Self {
        Self { cache, permission }
    }

    /// The cache this grant applies to.
    #[must_use]
    pub fn cache(&self) -> &CacheName {
        &self.cache
    }

    /// What it allows.
    #[must_use]
    pub fn permission(&self) -> Permission {
        self.permission
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No credentials were presented and the action needs them.
    Unauthenticated,
    /// Credentials were valid but do not cover the action.
    Forbidden,
}

/// The caller of a use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// No credentials.
    Anonymous,
    /// A token holder with its grants.
    Token {
        /// Admins manage tokens and have full access to every cache.
        admin: bool,
        /// Per-cache permissions.
        grants: Vec<Grant>,
    },
    /// An embedding program calling in-process; fully trusted.
    System,
}

impl Principal {
    /// Decide whether this caller may perform `action` on a cache.
    ///
    /// # Errors
    /// Returns [`Denied`] when the action is not allowed.
    pub fn authorize(
        &self,
        cache: &CacheName,
        visibility: Visibility,
        action: Action,
    ) -> Result<(), Denied> {
        if visibility == Visibility::Public && action == Action::Read {
            return Ok(());
        }
        match self {
            Self::System | Self::Token { admin: true, .. } => Ok(()),
            Self::Anonymous => Err(Denied::Unauthenticated),
            Self::Token { grants, .. } => grants
                .iter()
                .any(|g| g.cache == *cache && g.permission.allows(action))
                .then_some(())
                .ok_or(Denied::Forbidden),
        }
    }

    /// Decide whether this caller may manage tokens.
    ///
    /// # Errors
    /// Returns [`Denied`] for anyone but an admin or the embedding program.
    pub fn authorize_admin(&self) -> Result<(), Denied> {
        match self {
            Self::System | Self::Token { admin: true, .. } => Ok(()),
            Self::Anonymous => Err(Denied::Unauthenticated),
            Self::Token { .. } => Err(Denied::Forbidden),
        }
    }
}

/// A stored access token. The secret itself is never kept, only its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    id: String,
    name: String,
    hash: Digest,
    admin: bool,
    grants: Vec<Grant>,
    created_at: i64,
    revoked_at: Option<i64>,
}

impl TokenRecord {
    /// Assemble a record.
    #[must_use]
    pub fn new(
        id: String,
        name: String,
        hash: Digest,
        admin: bool,
        grants: Vec<Grant>,
        created_at: i64,
        revoked_at: Option<i64>,
    ) -> Self {
        Self {
            id,
            name,
            hash,
            admin,
            grants,
            created_at,
            revoked_at,
        }
    }

    /// Public identifier, safe to log and list.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Human label.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// SHA-256 of the secret.
    #[must_use]
    pub fn hash(&self) -> &Digest {
        &self.hash
    }

    /// Whether the token may manage tokens.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.admin
    }

    /// Per-cache permissions.
    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// Creation time, unix seconds.
    #[must_use]
    pub fn created_at(&self) -> i64 {
        self.created_at
    }

    /// Revocation time, unix seconds, if revoked.
    #[must_use]
    pub fn revoked_at(&self) -> Option<i64> {
        self.revoked_at
    }

    /// The caller this token authenticates as.
    #[must_use]
    pub fn into_principal(self) -> Principal {
        Principal::Token {
            admin: self.admin,
            grants: self.grants,
        }
    }
}

/// Prefix that marks a secret as a Procurator cache token.
const SECRET_PREFIX: &str = "pcr_";

/// A freshly minted token secret. Shown to the caller once, never stored.
pub struct TokenSecret(String);

impl TokenSecret {
    /// Build a secret from 32 random bytes.
    #[must_use]
    pub fn from_entropy(bytes: [u8; 32]) -> Self {
        let mut text = String::with_capacity(SECRET_PREFIX.len() + 64);
        text.push_str(SECRET_PREFIX);
        text.push_str(&hex::encode(bytes));
        Self(text)
    }

    /// The secret text to hand to the client.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The hash to store.
    #[must_use]
    pub fn hash(&self) -> Digest {
        Digest::of(self.0.as_bytes())
    }
}

impl fmt::Debug for TokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Why: secrets must never reach logs through a stray `{:?}`.
        f.write_str("TokenSecret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Denied, Grant, Permission, Principal, TokenSecret};
    use crate::domain::cache::{CacheName, Visibility};

    fn name(raw: &str) -> CacheName {
        CacheName::parse(raw).unwrap()
    }

    fn token(cache: &str, permission: Permission) -> Principal {
        Principal::Token {
            admin: false,
            grants: vec![Grant::new(name(cache), permission)],
        }
    }

    #[test]
    fn public_cache_reads_are_open_but_writes_are_not() {
        let cache = name("pub");
        let public = Visibility::Public;
        assert_eq!(
            Principal::Anonymous.authorize(&cache, public, Action::Read),
            Ok(())
        );
        assert_eq!(
            Principal::Anonymous.authorize(&cache, public, Action::Write),
            Err(Denied::Unauthenticated)
        );
        assert_eq!(
            token("pub", Permission::Read).authorize(&cache, public, Action::Write),
            Err(Denied::Forbidden)
        );
        assert_eq!(
            token("pub", Permission::Write).authorize(&cache, public, Action::Write),
            Ok(())
        );
    }

    #[test]
    fn private_cache_needs_a_grant_for_everything() {
        let cache = name("priv");
        let private = Visibility::Private;
        assert_eq!(
            Principal::Anonymous.authorize(&cache, private, Action::Read),
            Err(Denied::Unauthenticated)
        );
        assert_eq!(
            token("other", Permission::Write).authorize(&cache, private, Action::Read),
            Err(Denied::Forbidden)
        );
        assert_eq!(
            token("priv", Permission::Read).authorize(&cache, private, Action::Read),
            Ok(())
        );
        assert_eq!(
            token("priv", Permission::Write).authorize(&cache, private, Action::Read),
            Ok(())
        );
    }

    #[test]
    fn admins_and_system_bypass_grants() {
        let cache = name("priv");
        let admin = Principal::Token {
            admin: true,
            grants: Vec::new(),
        };
        assert_eq!(
            admin.authorize(&cache, Visibility::Private, Action::Write),
            Ok(())
        );
        assert_eq!(admin.authorize_admin(), Ok(()));
        assert_eq!(Principal::System.authorize_admin(), Ok(()));
        assert_eq!(
            token("priv", Permission::Write).authorize_admin(),
            Err(Denied::Forbidden)
        );
        assert_eq!(
            Principal::Anonymous.authorize_admin(),
            Err(Denied::Unauthenticated)
        );
    }

    #[test]
    fn secrets_hide_from_debug_and_hash_stably() {
        let secret = TokenSecret::from_entropy([7; 32]);
        assert!(secret.as_str().starts_with("pcr_"));
        assert_eq!(format!("{secret:?}"), "TokenSecret(..)");
        assert_eq!(secret.hash(), TokenSecret::from_entropy([7; 32]).hash());
    }
}
