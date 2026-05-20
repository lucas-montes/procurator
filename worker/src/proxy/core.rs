use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{
        Request, Response, StatusCode, Uri,
        header::{AUTHORIZATION, CONTENT_TYPE, HOST, HeaderMap, HeaderValue, LOCATION, SET_COOKIE},
    },
};
use http_body_util::BodyExt;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use tokio::time;

use crate::vmm::{Factory, Reader, Registry};

/// TCP port on which `opencode serve` listens inside every guest image.
///
/// This is fixed by the in-image systemd unit (`opencode-server` in
/// `nix/lib/diskVm.nix`) which invokes `opencode serve --port 4096`.
/// Changing this requires rebuilding the VM image, so it is not a runtime
/// configuration knob.
pub const OPENCODE_UPSTREAM_PORT: u16 = 4096;
const SESSION_COOKIE_NAME: &str = "pcr_session";

/// JWT claims the worker enforces on every proxied request.
///
/// `vm_id` binds the token to exactly one VM (one token, one VM); requests
/// whose extracted path id differs are rejected with 403. `exp` is required
/// and validated by `jsonwebtoken` so stolen tokens have a hard ceiling on
/// their lifetime — the control plane is expected to mint short-lived
/// tokens (minutes, not days).
///
/// `exp` is not read by our Rust code: `jsonwebtoken` enforces it during
/// `decode` when `validation.validate_exp` is on. The field still has to
/// be present in the struct so deserialization confirms it exists with
/// the right type.
#[derive(Debug, Deserialize)]
struct ProxyJwtClaims {
    vm_id: String,
    #[allow(dead_code)]
    exp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmProxyRoute {
    vm_id: String,
    upstream_path_and_query: String,
}

#[derive(Clone)]
pub struct ProxyState<F: Factory> {
    pub registry: Registry<F, Reader>,
    pub client: Client<HttpConnector, Body>,
    pub jwt_hs256_secret: String,
    pub base_domain: String,
    pub runtime_settings: ProxyRuntimeSettings,
}

#[derive(Debug, Deserialize)]
pub struct AuthBootstrapQuery {
    token: String,
    next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyRouteError {
    MissingHost,
    InvalidHost,
    MissingVmSubdomain,
    BaseDomainMismatch,
}

/// # Errors
///
/// Returns [`ProxyRouteError::MissingHost`] when no `Host` header is present,
/// [`ProxyRouteError::InvalidHost`] when the `Host` value is malformed,
/// [`ProxyRouteError::MissingVmSubdomain`] when the host is exactly the base domain,
/// or [`ProxyRouteError::BaseDomainMismatch`] when it does not end with the base domain.
pub fn extract_vm_proxy_route(
    host_header: Option<&HeaderValue>,
    base_domain: &str,
    path_and_query: &str,
) -> Result<VmProxyRoute, ProxyRouteError> {
    let host_value = host_header
        .ok_or(ProxyRouteError::MissingHost)?
        .to_str()
        .map_err(|_| ProxyRouteError::InvalidHost)?
        .trim();

    if host_value.is_empty() {
        return Err(ProxyRouteError::MissingHost);
    }

    if host_value.starts_with('[') {
        return Err(ProxyRouteError::InvalidHost);
    }

    let host = host_value
        .trim_end_matches('.')
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let base_domain = base_domain.trim_end_matches('.').to_ascii_lowercase();
    if host == base_domain {
        return Err(ProxyRouteError::MissingVmSubdomain);
    }

    let suffix = format!(".{base_domain}");
    if !host.ends_with(&suffix) {
        return Err(ProxyRouteError::BaseDomainMismatch);
    }

    let vm_id = host.strip_suffix(&suffix).unwrap_or_default().to_string();

    if vm_id.is_empty() {
        return Err(ProxyRouteError::MissingVmSubdomain);
    }

    let suffix = format!(".{base_domain}");
    if !host.ends_with(&suffix) {
        return Err(ProxyRouteError::BaseDomainMismatch);
    }

    let vm_id = host.strip_suffix(&suffix).unwrap_or_default().to_string();

    if vm_id.is_empty() {
        return Err(ProxyRouteError::MissingVmSubdomain);
    }

    Ok(VmProxyRoute {
        vm_id,
        upstream_path_and_query: path_and_query.to_string(),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct ProxyRuntimeSettings {
    upstream_request_timeout: Duration,
}

impl ProxyRuntimeSettings {
    pub fn from_timeout_millis(upstream_request_timeout_millis: u64) -> Self {
        Self {
            upstream_request_timeout: Duration::from_millis(upstream_request_timeout_millis),
        }
    }
}

pub async fn proxy_handler<F: Factory>(
    State(state): State<Arc<ProxyState<F>>>,
    request: Request<Body>,
) -> Result<Response<Body>, std::convert::Infallible> {
    Ok(proxy_vm_request(
        &state.registry,
        &state.client,
        &state.jwt_hs256_secret,
        &state.base_domain,
        request,
        OPENCODE_UPSTREAM_PORT,
        state.runtime_settings,
    )
    .await)
}

pub async fn auth_bootstrap<F: Factory>(
    State(state): State<Arc<ProxyState<F>>>,
    Query(query): Query<AuthBootstrapQuery>,
    headers: HeaderMap,
) -> Response<Body> {
    auth_bootstrap_response(
        headers.get(HOST),
        &state.base_domain,
        &state.jwt_hs256_secret,
        &query,
    )
}

pub async fn auth_logout<F: Factory>(
    State(state): State<Arc<ProxyState<F>>>,
    headers: HeaderMap,
) -> Response<Body> {
    auth_logout_response(headers.get(HOST), &state.base_domain)
}

fn auth_bootstrap_response(
    host_header: Option<&HeaderValue>,
    base_domain: &str,
    jwt_hs256_secret: &str,
    query: &AuthBootstrapQuery,
) -> Response<Body> {
    let token = query.token.trim();
    if token.is_empty() {
        return simple_error_response(StatusCode::UNAUTHORIZED, "missing token");
    }

    let Ok(route) = extract_vm_proxy_route(host_header, base_domain, "/") else {
        return simple_error_response(StatusCode::NOT_FOUND, "vm route not found");
    };

    let claims = match decode_token(token, jwt_hs256_secret) {
        Ok(claims) => claims,
        Err(AuthzOutcome::ForbiddenForVm) => {
            return simple_error_response(StatusCode::FORBIDDEN, "forbidden for vm");
        }
        Err(AuthzOutcome::MissingOrInvalidToken | AuthzOutcome::Authorized) => {
            return simple_error_response(StatusCode::UNAUTHORIZED, "missing or invalid token");
        }
    };

    if claims.vm_id != route.vm_id {
        return simple_error_response(StatusCode::FORBIDDEN, "forbidden for vm");
    }

    let Some(max_age) = cookie_max_age_seconds(claims.exp) else {
        return simple_error_response(StatusCode::UNAUTHORIZED, "expired token");
    };

    let cookie_domain = format!("{}.{}", route.vm_id, base_domain.trim_end_matches('.'));
    let cookie_value = format!(
        "{SESSION_COOKIE_NAME}={token}; Domain={cookie_domain}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}",
    );

    let location = query
        .next
        .as_deref()
        .and_then(valid_next_path)
        .unwrap_or("/");

    Response::builder()
        .status(StatusCode::FOUND)
        .header(LOCATION, location)
        .header(SET_COOKIE, cookie_value)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn auth_logout_response(host_header: Option<&HeaderValue>, base_domain: &str) -> Response<Body> {
    let Ok(route) = extract_vm_proxy_route(host_header, base_domain, "/") else {
        return simple_error_response(StatusCode::NOT_FOUND, "vm route not found");
    };

    let cookie_domain = format!("{}.{}", route.vm_id, base_domain.trim_end_matches('.'));
    let cookie_value = format!(
        "{SESSION_COOKIE_NAME}=; Domain={cookie_domain}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0",
    );

    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(SET_COOKIE, cookie_value)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn valid_next_path(next: &str) -> Option<&str> {
    if next.starts_with('/') && !next.starts_with("//") && !next.contains("://") {
        Some(next)
    } else {
        None
    }
}

fn cookie_max_age_seconds(exp: u64) -> Option<u64> {
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => return None,
    };

    exp.checked_sub(now)
}

#[allow(clippy::too_many_lines)]
pub async fn proxy_vm_request<F: Factory>(
    registry: &Registry<F, Reader>,
    client: &Client<HttpConnector, Body>,
    jwt_hs256_secret: &str,
    base_domain: &str,
    mut request: Request<Body>,
    upstream_port: u16,
    settings: ProxyRuntimeSettings,
) -> Response<Body> {
    let started_at = Instant::now();

    let Ok(route) = extract_vm_proxy_route(
        request.headers().get(HOST),
        base_domain,
        request
            .uri()
            .path_and_query()
            .map_or(request.uri().path(), |pq| pq.as_str()),
    ) else {
        let status = StatusCode::NOT_FOUND;
        let latency_ms =
            u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
        tracing::warn!(
            vm_id = "-",
            upstream = "-",
            %status,
            latency_ms,
            auth_result = "route_not_found",
            "Handled proxy request"
        );
        return simple_error_response(status, "vm route not found");
    };

    let (auth_outcome, auth_source) =
        authorize_vm_access(request.headers(), jwt_hs256_secret, &route.vm_id);
    let auth_result = match auth_outcome {
        AuthzOutcome::Authorized => match auth_source.expect("authorized must have source") {
            AuthSource::Bearer => "bearer",
            AuthSource::Cookie => "cookie",
        },
        AuthzOutcome::MissingOrInvalidToken => {
            let status = StatusCode::UNAUTHORIZED;
            let latency_ms =
                u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
            tracing::warn!(
                vm_id = %route.vm_id,
                upstream = "-",
                %status,
                latency_ms,
                auth_result = "missing_or_invalid_token",
                "Handled proxy request"
            );
            return simple_error_response(status, "missing or invalid token");
        }
        AuthzOutcome::ForbiddenForVm => {
            let status = StatusCode::FORBIDDEN;
            let latency_ms =
                u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
            tracing::warn!(
                vm_id = %route.vm_id,
                upstream = "-",
                %status,
                latency_ms,
                auth_result = "forbidden_for_vm",
                "Handled proxy request"
            );
            return simple_error_response(status, "forbidden for vm");
        }
    };

    let registry = registry.clone().get().await;
    let vm_ip = registry.get(&route.vm_id).map(crate::vmm::Handle::ip);

    let Some(vm_ip) = vm_ip else {
        let status = StatusCode::NOT_FOUND;
        let latency_ms =
            u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
        tracing::warn!(
            vm_id = %route.vm_id,
            upstream = "-",
            %status,
            latency_ms,
            auth_result,
            "Handled proxy request"
        );
        return simple_error_response(status, "unknown vm id");
    };

    let Ok(upstream_uri) = Uri::builder()
        .scheme("http")
        .authority(format!("{vm_ip}:{upstream_port}"))
        .path_and_query(route.upstream_path_and_query.as_str())
        .build()
    else {
        let status = StatusCode::BAD_GATEWAY;
        let latency_ms =
            u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
        tracing::error!(
            vm_id = %route.vm_id,
            vm_ip,
            upstream_port,
            path_and_query = %route.upstream_path_and_query,
            %status,
            latency_ms,
            auth_result,
            "Handled proxy request"
        );
        return simple_error_response(status, "failed to build upstream uri");
    };

    *request.uri_mut() = upstream_uri;
    request.headers_mut().remove(HOST);

    let upstream_response: Result<hyper::Response<hyper::body::Incoming>, _> = if let Ok(result) =
        time::timeout(settings.upstream_request_timeout, client.request(request)).await
    {
        result
    } else {
        let status = StatusCode::GATEWAY_TIMEOUT;
        let latency_ms =
            u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
        tracing::warn!(
            vm_id = %route.vm_id,
            vm_ip,
            upstream_port,
            path_and_query = %route.upstream_path_and_query,
            %status,
            latency_ms,
            auth_result,
            "Handled proxy request"
        );
        return simple_error_response(status, "upstream request timed out");
    };

    let response = match upstream_response {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let body = Body::from_stream(body.into_data_stream());
            Response::from_parts(parts, body)
        }
        Err(err) => {
            let status = StatusCode::BAD_GATEWAY;
            let latency_ms =
                u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
            tracing::error!(
                vm_id = %route.vm_id,
                vm_ip,
            upstream_port,
            path_and_query = %route.upstream_path_and_query,
                %status,
                latency_ms,
                auth_result,
                ?err,
                "Handled proxy request"
            );
            return simple_error_response(status, "upstream request failed");
        }
    };

    let status = response.status();
    let latency_ms = u64::try_from(started_at.elapsed().as_millis()).expect("latency fits in u64");
    tracing::info!(
        vm_id = %route.vm_id,
        vm_ip,
            upstream_port,
            path_and_query = %route.upstream_path_and_query,
        %status,
        latency_ms,
        auth_result,
        "Handled proxy request"
    );

    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthzOutcome {
    Authorized,
    MissingOrInvalidToken,
    ForbiddenForVm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthSource {
    Bearer,
    Cookie,
}

fn authorize_vm_access(
    headers: &HeaderMap,
    jwt_hs256_secret: &str,
    vm_id: &str,
) -> (AuthzOutcome, Option<AuthSource>) {
    let Some((token, source)) = request_token(headers) else {
        return (AuthzOutcome::MissingOrInvalidToken, None);
    };
    (
        verify_vm_access(token, jwt_hs256_secret, vm_id),
        Some(source),
    )
}

fn verify_vm_access(token: &str, jwt_hs256_secret: &str, vm_id: &str) -> AuthzOutcome {
    let claims = match decode_token(token, jwt_hs256_secret) {
        Ok(claims) => claims,
        Err(outcome) => return outcome,
    };

    if claims.vm_id == vm_id {
        AuthzOutcome::Authorized
    } else {
        AuthzOutcome::ForbiddenForVm
    }
}

/// Returns the JWT supplied with the request.
///
/// The proxy is consumed by the opencode SDKs, the TUI, and ad-hoc
/// `curl`/scripts — all of which send `Authorization: Bearer <jwt>`. The
/// guest opencode server has no browser-facing HTML, so cookie-based auth
/// is not part of the design.
fn request_token(headers: &HeaderMap) -> Option<(&str, AuthSource)> {
    request_bearer_token(headers)
        .map(|token| (token, AuthSource::Bearer))
        .or_else(|| request_cookie_token(headers).map(|token| (token, AuthSource::Cookie)))
}

fn request_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn request_cookie_token(headers: &HeaderMap) -> Option<&str> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie_header
        .split(';')
        .map(str::trim)
        .find_map(|value| value.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")))
}

fn decode_token(token: &str, jwt_hs256_secret: &str) -> Result<ProxyJwtClaims, AuthzOutcome> {
    let mut validation = Validation::new(Algorithm::HS256);
    // Require `exp` and have `jsonwebtoken` enforce it; a missing or
    // past `exp` causes `decode` to fail and we map that to 401.
    validation.validate_exp = true;
    validation.required_spec_claims = ["exp".to_string()].into_iter().collect();

    decode::<ProxyJwtClaims>(
        token,
        &DecodingKey::from_secret(jwt_hs256_secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|_| AuthzOutcome::MissingOrInvalidToken)
}

fn simple_error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(message))
        .unwrap_or_else(|_| Response::new(Body::from(message)))
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::Arc};

    use axum::{
        Router,
        body::Body,
        http::{
            Method, Request, Response, StatusCode,
            header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue},
        },
        routing::any,
    };
    use http_body_util::BodyExt;
    use hyper_util::rt::TokioExecutor;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use tokio::{net::TcpListener, sync::RwLock, time::sleep};

    use super::*;

    #[derive(Clone)]
    struct FakeVmHandle {
        ip: String,
    }

    impl crate::vmm::Handle for FakeVmHandle {
        fn ip(&self) -> &str {
            &self.ip
        }

        async fn start(&self) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn delete(self) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn health(&self) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn pause(&self) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn resume(&self) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn snapshot(
            &self,
            _destination: std::path::PathBuf,
        ) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }

        async fn backup_disk(
            &self,
            _destination: std::path::PathBuf,
        ) -> Result<(), crate::vmm::HandleError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FakeCreateVmSpec;

    impl<'a> TryFrom<commands::common_capnp::vm_spec::Reader<'a, ::capnp::any_pointer::Owned>>
        for FakeCreateVmSpec
    {
        type Error = capnp::Error;

        fn try_from(
            _: commands::common_capnp::vm_spec::Reader<'a, ::capnp::any_pointer::Owned>,
        ) -> Result<Self, Self::Error> {
            Ok(Self)
        }
    }

    #[derive(Clone, Debug)]
    struct FakeFactory;

    #[derive(Serialize)]
    struct TestClaims {
        vm_id: String,
        exp: u64,
    }

    const TEST_PROXY_JWT_SECRET: &str = "proxy-secret";
    /// Long enough that no non-timeout test will hit it.
    const TEST_TIMEOUT_MILLIS: u64 = 60_000;
    /// Far-future expiry used for tests that just need a valid token.
    /// Year 2099 in seconds since the epoch.
    const TEST_FAR_FUTURE_EXP: u64 = 4_070_908_800;

    impl crate::vmm::Factory for FakeFactory {
        type VmHandle = FakeVmHandle;
        type Config = serde_json::Value;
        type BackendConfig = ::capnp::any_pointer::Owned;
        type CreateVmSpec<'a> = FakeCreateVmSpec;

        fn create_id() -> String {
            "fake-id".to_string()
        }

        async fn create_vm(
            &self,
            _source: Self::CreateVmSpec<'_>,
        ) -> Result<crate::vmm::CreateCommand<Self>, crate::vmm::Error>
        where
            Self: Sized,
        {
            unimplemented!("not needed for proxy tests")
        }
    }

    /// Mint a JWT bound to `vm_id` with a far-future `exp`. Used for
    /// the happy path of most tests; the exp-edge cases mint directly.
    fn bearer_token_for_vm(vm_id: &str, secret: &str) -> String {
        let claims = TestClaims {
            vm_id: vm_id.to_string(),
            exp: TEST_FAR_FUTURE_EXP,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token encoded");

        format!("Bearer {token}")
    }

    fn raw_token_for_vm(vm_id: &str, secret: &str) -> String {
        let claims = TestClaims {
            vm_id: vm_id.to_string(),
            exp: TEST_FAR_FUTURE_EXP,
        };

        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token encoded")
    }

    #[test]
    fn extracts_vm_route_from_host_and_preserves_path() {
        let host = HeaderValue::from_static("vm-123.vm.example.test");
        let route = extract_vm_proxy_route(Some(&host), "vm.example.test", "/doc?x=1")
            .expect("route parsed");
        assert_eq!(route.vm_id, "vm-123");
        assert_eq!(route.upstream_path_and_query, "/doc?x=1");
    }

    #[test]
    fn rejects_missing_vm_subdomain() {
        let host = HeaderValue::from_static("vm.example.test");
        let err = extract_vm_proxy_route(Some(&host), "vm.example.test", "/")
            .expect_err("expected error");
        assert_eq!(err, ProxyRouteError::MissingVmSubdomain);
    }

    #[test]
    fn rejects_wrong_base_domain() {
        let host = HeaderValue::from_static("vm-123.other.test");
        let err = extract_vm_proxy_route(Some(&host), "vm.example.test", "/")
            .expect_err("expected error");
        assert_eq!(err, ProxyRouteError::BaseDomainMismatch);
    }

    #[test]
    fn request_token_reads_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer xyz"));
        assert_eq!(request_token(&headers), Some(("xyz", AuthSource::Bearer)));
    }

    #[test]
    fn request_token_reads_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("pcr_session=cookie-jwt"));
        assert_eq!(
            request_token(&headers),
            Some(("cookie-jwt", AuthSource::Cookie))
        );
    }

    #[test]
    fn bootstrap_sets_cookie_and_redirects() {
        let token = raw_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET);
        let host = HeaderValue::from_static("vm-123.vm.example.test");
        let query = AuthBootstrapQuery {
            token: token.clone(),
            next: Some("/console".to_string()),
        };

        let response = auth_bootstrap_response(
            Some(&host),
            "vm.example.test",
            TEST_PROXY_JWT_SECRET,
            &query,
        );

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(LOCATION)
                .expect("location")
                .to_str()
                .expect("location str"),
            "/console"
        );

        let set_cookie = response
            .headers()
            .get(SET_COOKIE)
            .expect("set-cookie")
            .to_str()
            .expect("cookie str");
        assert!(set_cookie.contains("pcr_session="));
        assert!(set_cookie.contains("Domain=vm-123.vm.example.test"));
        assert!(set_cookie.contains(&token));
    }

    #[test]
    fn bootstrap_rejects_token_for_other_vm() {
        let token = raw_token_for_vm("vm-999", TEST_PROXY_JWT_SECRET);
        let host = HeaderValue::from_static("vm-123.vm.example.test");
        let query = AuthBootstrapQuery { token, next: None };

        let response = auth_bootstrap_response(
            Some(&host),
            "vm.example.test",
            TEST_PROXY_JWT_SECRET,
            &query,
        );

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn bootstrap_rejects_unsafe_next_param() {
        let token = raw_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET);
        let host = HeaderValue::from_static("vm-123.vm.example.test");
        let query = AuthBootstrapQuery {
            token,
            next: Some("//evil.example".to_string()),
        };

        let response = auth_bootstrap_response(
            Some(&host),
            "vm.example.test",
            TEST_PROXY_JWT_SECRET,
            &query,
        );

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(LOCATION)
                .expect("location")
                .to_str()
                .expect("location str"),
            "/"
        );
    }

    #[tokio::test]
    async fn proxies_request_to_vm_upstream_preserving_path() {
        let received_path = Arc::new(RwLock::new(String::new()));
        let received_path_clone = Arc::clone(&received_path);

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind upstream listener");
        let upstream_addr = listener.local_addr().expect("upstream addr");
        let server_task = tokio::spawn(async move {
            let app = Router::new().fallback(any(move |request: Request<Body>| {
                let received_path = Arc::clone(&received_path_clone);
                async move {
                    *received_path.write().await = request.uri().path_and_query().map_or_else(
                        || request.uri().path().to_string(),
                        |pq| pq.as_str().to_string(),
                    );

                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "text/event-stream")
                        .body(Body::from("event: ping\ndata: ok\n\n"))
                        .expect("response")
                }
            }));

            axum::serve(listener, app).await.expect("upstream server");
        });

        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, mut writer_registry) = Registry::<FakeFactory, _>::new(db).split();

        writer_registry
            .insert(
                "vm-123".to_string(),
                FakeVmHandle {
                    ip: upstream_addr.ip().to_string(),
                },
            )
            .await;

        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        let request = Request::builder()
            .method(Method::GET)
            .uri("/doc?x=1")
            .header("host", "vm-123.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            upstream_addr.port(),
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .expect("content-type")
                .to_str()
                .expect("header str"),
            "text/event-stream"
        );

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body bytes")
            .to_bytes();
        assert_eq!(&body[..], b"event: ping\ndata: ok\n\n");

        assert_eq!(&*received_path.read().await, "/doc?x=1");

        server_task.abort();
    }

    #[tokio::test]
    async fn returns_404_when_vm_id_is_unknown() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, _) = Registry::<FakeFactory, _>::new(db).split();
        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());

        let request = Request::builder()
            .method(Method::GET)
            .uri("/doc")
            .header("host", "missing.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("missing", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn returns_502_when_upstream_connection_fails() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, mut writer_registry) = Registry::<FakeFactory, _>::new(db).split();

        writer_registry
            .insert(
                "vm-123".to_string(),
                FakeVmHandle {
                    ip: "127.0.0.1".to_string(),
                },
            )
            .await;

        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        let request = Request::builder()
            .method(Method::GET)
            .uri("/doc")
            .header("host", "vm-123.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn returns_504_when_upstream_request_times_out() {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind upstream listener");
        let upstream_addr = listener.local_addr().expect("upstream addr");
        let server_task = tokio::spawn(async move {
            let app = Router::new().fallback(any(move |_request: Request<Body>| async move {
                sleep(Duration::from_millis(200)).await;
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from("late response"))
                    .expect("response")
            }));

            axum::serve(listener, app).await.expect("upstream server");
        });

        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, mut writer_registry) = Registry::<FakeFactory, _>::new(db).split();

        writer_registry
            .insert(
                "vm-123".to_string(),
                FakeVmHandle {
                    ip: upstream_addr.ip().to_string(),
                },
            )
            .await;

        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        let request = Request::builder()
            .method(Method::GET)
            .uri("/event")
            .header("host", "vm-123.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            upstream_addr.port(),
            ProxyRuntimeSettings::from_timeout_millis(50),
        )
        .await;

        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);

        server_task.abort();
    }

    #[test]
    fn authz_rejects_missing_authorization_header() {
        let headers = HeaderMap::new();

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, (AuthzOutcome::MissingOrInvalidToken, None));
    }

    #[test]
    fn authz_rejects_non_bearer_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic abc123"));

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, (AuthzOutcome::MissingOrInvalidToken, None));
    }

    #[test]
    fn authz_rejects_bad_signature_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-123", "wrong-secret"))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(
            result,
            (
                AuthzOutcome::MissingOrInvalidToken,
                Some(AuthSource::Bearer)
            )
        );
    }

    #[test]
    fn authz_rejects_malformed_vm_id_claim() {
        // `vm_id` must be a string; an array fails deserialization.
        let malformed_token = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "vm_id": ["vm-123"], "exp": TEST_FAR_FUTURE_EXP }),
            &EncodingKey::from_secret(TEST_PROXY_JWT_SECRET.as_bytes()),
        )
        .expect("token encoded");

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {malformed_token}"))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(
            result,
            (
                AuthzOutcome::MissingOrInvalidToken,
                Some(AuthSource::Bearer)
            )
        );
    }

    #[test]
    fn authz_rejects_token_missing_exp_claim() {
        // No `exp` — the worker requires it.
        let token_without_exp = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "vm_id": "vm-123" }),
            &EncodingKey::from_secret(TEST_PROXY_JWT_SECRET.as_bytes()),
        )
        .expect("token encoded");

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token_without_exp}"))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(
            result,
            (
                AuthzOutcome::MissingOrInvalidToken,
                Some(AuthSource::Bearer)
            )
        );
    }

    #[test]
    fn authz_rejects_expired_token() {
        // `exp` in the past must fail verification.
        let expired_token = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "vm_id": "vm-123", "exp": 1_u64 }),
            &EncodingKey::from_secret(TEST_PROXY_JWT_SECRET.as_bytes()),
        )
        .expect("token encoded");

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {expired_token}"))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(
            result,
            (
                AuthzOutcome::MissingOrInvalidToken,
                Some(AuthSource::Bearer)
            )
        );
    }

    #[test]
    fn authz_rejects_token_bound_to_different_vm() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-999", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(
            result,
            (AuthzOutcome::ForbiddenForVm, Some(AuthSource::Bearer))
        );
    }

    #[test]
    fn authz_allows_valid_token_bound_to_vm() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-123", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, (AuthzOutcome::Authorized, Some(AuthSource::Bearer)));
    }

    #[tokio::test]
    async fn returns_401_when_authorization_header_is_missing() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, _) = Registry::<FakeFactory, _>::new(db).split();
        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());

        let request = Request::builder()
            .method(Method::GET)
            .uri("/doc")
            .header("host", "vm-123.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn returns_403_when_token_lacks_vm_access() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, mut writer_registry) = Registry::<FakeFactory, _>::new(db).split();

        writer_registry
            .insert(
                "vm-123".to_string(),
                FakeVmHandle {
                    ip: "127.0.0.1".to_string(),
                },
            )
            .await;

        let client = Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        let request = Request::builder()
            .method(Method::GET)
            .uri("/doc")
            .header("host", "vm-123.vm.example.test")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm("vm-999", TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            "vm.example.test",
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
