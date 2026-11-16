//! An [`Access`] that lets everyone read everything.
//! Why: repohub has no credentials, members or visibility yet, so nothing
//! can tell users apart. Fine for a single-user homelab; replace it once
//! repohub can answer who may read a repo.

use std::future::{Future, ready};

use crate::application::ports::{Access, AccessError};
use crate::domain::access::{Credential, Principal};
use crate::domain::run::RepoName;

/// Everyone is anonymous and reads every repo.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAccess;

impl Access for OpenAccess {
    fn authenticate(
        &self,
        _credential: Credential<'_>,
    ) -> impl Future<Output = Result<Principal, AccessError>> + Send {
        ready(Ok(Principal::Anonymous))
    }

    fn can_read(
        &self,
        _principal: &Principal,
        _repo: &RepoName,
    ) -> impl Future<Output = Result<bool, AccessError>> + Send {
        ready(Ok(true))
    }
}
