use std::borrow::Cow;

/// Context extracted from incoming proxy request metadata.
#[derive(Debug, Clone)]
pub struct ProxyRequestContext<'a> {
    pub vm_id: Cow<'a, str>,
    pub path_and_query: Cow<'a, str>,
}

/// Result of authn/authz evaluation for an incoming proxy request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    Authorized,
    MissingOrInvalidToken,
    ForbiddenForVm,
}

/// Canonical upstream destination for a VM-specific proxied request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyUpstreamTarget {
    pub vm_id: String,
    pub upstream_authority: String,
    pub rewritten_path_and_query: String,
}

/// Resolves VM IDs to their current upstream authority (e.g. `10.0.0.5:4096`).
pub trait VmTargetResolver: Send + Sync {
    type Error;

    /// Resolve a VM ID to its upstream authority address.
    ///
    /// # Errors
    ///
    /// Returns the resolver's error type if the VM cannot be looked up.
    fn resolve_upstream_authority(&self, vm_id: &str) -> Result<Option<String>, Self::Error>;
}

/// Validates request credentials and VM-scoped access permissions.
pub trait TokenAuthorizer: Send + Sync {
    type Error;

    /// Authorize a bearer token for access to a specific VM.
    ///
    /// # Errors
    ///
    /// Returns the authorizer's error type if validation fails.
    fn authorize_vm_access(&self, token: &str, vm_id: &str) -> Result<AuthResult, Self::Error>;
}

/// Rewrites public proxy path into upstream path semantics.
pub trait ProxyRequestRewriter: Send + Sync {
    type Error;

    /// Rewrite a proxy request context into an upstream target.
    ///
    /// # Errors
    ///
    /// Returns the rewriter's error type if the rewrite fails.
    fn rewrite(&self, input: &ProxyRequestContext<'_>) -> Result<ProxyUpstreamTarget, Self::Error>;
}

/// Abstracts upstream transport so handler orchestration is testable without networking.
pub trait ProxyUpstreamTransport: Send + Sync {
    type Error;
    type Request;
    type Response;

    /// Send a request to the upstream and return the response.
    ///
    /// # Errors
    ///
    /// Returns the transport's error type if the request fails.
    fn send(&self, request: Self::Request) -> Result<Self::Response, Self::Error>;
}
