//! Authentication and token administration.

use std::future::Future;

use super::Service;
use crate::application::ServiceError;
use crate::application::ports::{BlobStore, Clock, Entropy, Index};
use crate::domain::access::{Grant, Permission, Principal, TokenRecord, TokenSecret};
use crate::domain::cache::CacheName;
use crate::domain::digest::Digest;

/// Longest accepted token label.
const MAX_TOKEN_NAME_LEN: usize = 64;

/// A request to mint a token.
#[derive(Debug)]
pub struct NewToken<'a> {
    name: &'a str,
    admin: bool,
    grants: Vec<(&'a str, &'a str)>,
}

impl<'a> NewToken<'a> {
    /// Describe a token: a label, whether it is an admin, and
    /// `(cache, permission)` pairs in wire form (`read` | `write`).
    #[must_use]
    pub fn new(name: &'a str, admin: bool, grants: Vec<(&'a str, &'a str)>) -> Self {
        Self {
            name,
            admin,
            grants,
        }
    }
}

impl<B, I, E> Service<B, I, E>
where
    B: BlobStore,
    I: Index,
    E: Clock + Entropy,
{
    /// Turn a presented secret into a caller. No secret means anonymous; an
    /// unknown or revoked secret is an error, never a silent downgrade.
    ///
    /// # Errors
    /// Fails with `Unauthenticated` for an unknown or revoked secret.
    pub fn authenticate(
        &self,
        secret: Option<&str>,
    ) -> impl Future<Output = Result<Principal, ServiceError>> + Send {
        async move {
            let Some(secret) = secret else {
                return Ok(Principal::Anonymous);
            };
            // Why: comparing hashes, not secrets, keeps lookups timing-safe.
            let hash = Digest::of(secret.as_bytes());
            if self.root_token == Some(hash) {
                return Ok(Principal::Token {
                    admin: true,
                    grants: Vec::new(),
                });
            }
            match self.index.token_by_hash(&hash).await? {
                Some(token) if token.revoked_at().is_none() => Ok(token.into_principal()),
                _ => Err(ServiceError::Unauthenticated),
            }
        }
    }

    /// Mint a token. Returns its record and the secret, which is shown once.
    ///
    /// # Errors
    /// Fails when the caller is not an admin or the request names an unknown
    /// cache or permission.
    pub fn create_token(
        &self,
        who: &Principal,
        request: NewToken<'_>,
    ) -> impl Future<Output = Result<(TokenRecord, TokenSecret), ServiceError>> + Send {
        async move {
            who.authorize_admin()?;
            if request.name.is_empty() || request.name.len() > MAX_TOKEN_NAME_LEN {
                return Err(ServiceError::Invalid("token name must be 1-64 characters"));
            }
            let mut grants = Vec::with_capacity(request.grants.len());
            for (cache, permission) in request.grants {
                // Why: a grant on a cache that does not exist is always a typo.
                self.spec(cache)?;
                let cache = CacheName::parse(cache).map_err(|_| ServiceError::UnknownCache)?;
                let permission = Permission::parse(permission)
                    .ok_or(ServiceError::Invalid("permission must be read or write"))?;
                grants.push(Grant::new(cache, permission));
            }
            let mut entropy = [0_u8; 32];
            self.env.fill(&mut entropy);
            let secret = TokenSecret::from_entropy(entropy);
            let mut id = [0_u8; 8];
            self.env.fill(&mut id);
            let record = TokenRecord::new(
                hex::encode(id),
                String::from(request.name),
                secret.hash(),
                request.admin,
                grants,
                self.env.now(),
                None,
            );
            self.index.insert_token(&record).await?;
            Ok((record, secret))
        }
    }

    /// Every token, revoked ones included.
    ///
    /// # Errors
    /// Fails when the caller is not an admin.
    pub fn list_tokens(
        &self,
        who: &Principal,
    ) -> impl Future<Output = Result<Vec<TokenRecord>, ServiceError>> + Send {
        async move {
            who.authorize_admin()?;
            Ok(self.index.tokens().await?)
        }
    }

    /// Revoke a token by id; `false` when no live token has that id.
    ///
    /// # Errors
    /// Fails when the caller is not an admin.
    pub fn revoke_token(
        &self,
        who: &Principal,
        id: &str,
    ) -> impl Future<Output = Result<bool, ServiceError>> + Send {
        async move {
            who.authorize_admin()?;
            Ok(self.index.revoke_token(id, self.env.now()).await?)
        }
    }
}
