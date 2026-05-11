use std::time::{Duration, Instant};

use hyper::{
    Body, Client, Request, Response, StatusCode, Uri,
    client::HttpConnector,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use tokio::time;

use crate::vmm::{Factory, Handle, Reader, Registry};

/// TCP port on which `opencode serve` listens inside every guest image.
///
/// This is fixed by the in-image systemd unit (`opencode-server` in
/// `nix/lib/diskVm.nix`) which invokes `opencode serve --port 4096`.
/// Changing this requires rebuilding the VM image, so it is not a runtime
/// configuration knob.
pub const OPENCODE_UPSTREAM_PORT: u16 = 4096;

#[derive(Debug, Deserialize)]
struct ProxyJwtClaims {
    vm_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmProxyRoute {
    vm_id: String,
    upstream_path_and_query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyRouteError {
    NotVmRoute,
    MissingVmId,
}

/// # Errors
///
/// Returns [`ProxyRouteError::NotVmRoute`] if the path does not start with `/vm/`,
/// or [`ProxyRouteError::MissingVmId`] if no VM identifier is present after the prefix.
pub fn extract_vm_proxy_route(path_and_query: &str) -> Result<VmProxyRoute, ProxyRouteError> {
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, query),
        None => (path_and_query, ""),
    };

    let remainder = path
        .strip_prefix("/vm/")
        .ok_or(ProxyRouteError::NotVmRoute)?;

    if remainder.is_empty() {
        return Err(ProxyRouteError::MissingVmId);
    }

    let mut segments = remainder.splitn(2, '/');
    let vm_id = segments.next().unwrap_or_default();
    if vm_id.is_empty() {
        return Err(ProxyRouteError::MissingVmId);
    }

    let trailing = segments.next();
    let rewritten_path = match trailing {
        Some("") | None => "/".to_string(),
        Some(rest) => format!("/{rest}"),
    };

    let upstream_path_and_query = if query.is_empty() {
        rewritten_path
    } else {
        format!("{rewritten_path}?{query}")
    };

    Ok(VmProxyRoute {
        vm_id: vm_id.to_string(),
        upstream_path_and_query,
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

pub async fn proxy_vm_request<F: Factory>(
    registry: &Registry<F, Reader>,
    client: &Client<HttpConnector, Body>,
    jwt_hs256_secret: &str,
    mut request: Request<Body>,
    upstream_port: u16,
    settings: ProxyRuntimeSettings,
) -> Response<Body> {
    let started_at = Instant::now();

    let Ok(route) = extract_vm_proxy_route(
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

    let auth_result = match authorize_vm_access(request.headers(), jwt_hs256_secret, &route.vm_id) {
        AuthzOutcome::Authorized => "authorized",
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
    let vm_ip = registry.get(&route.vm_id).map(|handle| handle.ip());

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
    request.headers_mut().remove(hyper::header::HOST);

    let upstream_response = match time::timeout(
        settings.upstream_request_timeout,
        client.request(request),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
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
        }
    };

    let response = match upstream_response {
        Ok(response) => response,
        Err(err) => {
            let status = if err.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            };
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

fn authorize_vm_access(headers: &HeaderMap, jwt_hs256_secret: &str, vm_id: &str) -> AuthzOutcome {
    let Some(token) = bearer_token(headers) else {
        return AuthzOutcome::MissingOrInvalidToken;
    };

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    validation.required_spec_claims.clear();

    let decoded = decode::<ProxyJwtClaims>(
        token,
        &DecodingKey::from_secret(jwt_hs256_secret.as_bytes()),
        &validation,
    );

    let claims = match decoded {
        Ok(data) => data.claims,
        Err(_) => return AuthzOutcome::MissingOrInvalidToken,
    };

    if claims
        .vm_ids
        .iter()
        .any(|allowed_vm_id| allowed_vm_id == vm_id)
    {
        AuthzOutcome::Authorized
    } else {
        AuthzOutcome::ForbiddenForVm
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let header_value = headers.get(AUTHORIZATION)?;
    let raw = header_value.to_str().ok()?;
    raw.strip_prefix("Bearer ")
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
    use std::{convert::Infallible, net::SocketAddr, sync::Arc};

    use hyper::{
        Method, Server,
        body::to_bytes,
        header::{AUTHORIZATION, HeaderMap, HeaderValue},
        service::make_service_fn,
        service::service_fn,
    };
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use tokio::{sync::RwLock, time::sleep};

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
        vm_ids: Vec<String>,
    }

    const TEST_PROXY_JWT_SECRET: &str = "proxy-secret";
    /// Long enough that no non-timeout test will hit it.
    const TEST_TIMEOUT_MILLIS: u64 = 60_000;

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

    fn bearer_token_for_vm_ids(vm_ids: &[&str], secret: &str) -> String {
        let claims = TestClaims {
            vm_ids: vm_ids.iter().map(|vm_id| (*vm_id).to_string()).collect(),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token encoded");

        format!("Bearer {token}")
    }

    #[test]
    fn extracts_vm_route_and_rewrites_path() {
        let route = extract_vm_proxy_route("/vm/vm-123/doc?x=1").expect("route parsed");
        assert_eq!(route.vm_id, "vm-123");
        assert_eq!(route.upstream_path_and_query, "/doc?x=1");
    }

    #[test]
    fn extracts_vm_route_root_path() {
        let route = extract_vm_proxy_route("/vm/vm-123").expect("route parsed");
        assert_eq!(route.vm_id, "vm-123");
        assert_eq!(route.upstream_path_and_query, "/");
    }

    #[test]
    fn rejects_non_vm_route() {
        let err = extract_vm_proxy_route("/health").expect_err("expected error");
        assert_eq!(err, ProxyRouteError::NotVmRoute);
    }

    #[tokio::test]
    async fn proxies_request_to_vm_upstream_with_prefix_strip() {
        let received_path = Arc::new(RwLock::new(String::new()));
        let received_path_clone = Arc::clone(&received_path);

        let server = Server::bind(&SocketAddr::from(([127, 0, 0, 1], 0))).serve(make_service_fn(
            move |_| {
                let received_path = Arc::clone(&received_path_clone);
                async move {
                    Ok::<_, Infallible>(service_fn(move |request| {
                        let received_path = Arc::clone(&received_path);
                        async move {
                            *received_path.write().await =
                                request.uri().path_and_query().map_or_else(
                                    || request.uri().path().to_string(),
                                    |pq| pq.as_str().to_string(),
                                );

                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .header(CONTENT_TYPE, "text/event-stream")
                                    .body(Body::from("event: ping\ndata: ok\n\n"))
                                    .expect("response"),
                            )
                        }
                    }))
                }
            },
        ));

        let upstream_addr = server.local_addr();
        let server_task = tokio::spawn(server);

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

        let client = Client::new();
        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/vm-123/doc?x=1")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-123"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
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

        let body = to_bytes(response.into_body()).await.expect("body bytes");
        assert_eq!(&body[..], b"event: ping\ndata: ok\n\n");

        assert_eq!(&*received_path.read().await, "/doc?x=1");

        server_task.abort();
    }

    #[tokio::test]
    async fn returns_404_when_vm_id_is_unknown() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, _) = Registry::<FakeFactory, _>::new(db).split();
        let client = Client::new();

        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/missing/doc")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(
                &["missing"],
                TEST_PROXY_JWT_SECRET,
            ))
            .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
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

        let client = Client::new();
        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/vm-123/doc")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-123"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn returns_504_when_upstream_request_times_out() {
        let server = Server::bind(&SocketAddr::from(([127, 0, 0, 1], 0))).serve(make_service_fn(
            move |_| async move {
                Ok::<_, Infallible>(service_fn(move |_request| async move {
                    sleep(Duration::from_millis(200)).await;
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from("late response"))
                            .expect("response"),
                    )
                }))
            },
        ));

        let upstream_addr = server.local_addr();
        let server_task = tokio::spawn(server);

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

        let client = Client::new();
        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/vm-123/event")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-123"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
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

        assert_eq!(result, AuthzOutcome::MissingOrInvalidToken);
    }

    #[test]
    fn authz_rejects_non_bearer_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic abc123"));

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, AuthzOutcome::MissingOrInvalidToken);
    }

    #[test]
    fn authz_rejects_bad_signature_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-123"], "wrong-secret"))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, AuthzOutcome::MissingOrInvalidToken);
    }

    #[test]
    fn authz_rejects_malformed_vm_ids_claim() {
        let malformed_token = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "vm_ids": "vm-123" }),
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

        assert_eq!(result, AuthzOutcome::MissingOrInvalidToken);
    }

    #[test]
    fn authz_rejects_valid_token_without_vm_membership() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-999"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, AuthzOutcome::ForbiddenForVm);
    }

    #[test]
    fn authz_allows_valid_token_with_vm_membership() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-123"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let result = authorize_vm_access(&headers, TEST_PROXY_JWT_SECRET, "vm-123");

        assert_eq!(result, AuthzOutcome::Authorized);
    }

    #[tokio::test]
    async fn returns_401_when_authorization_header_is_missing() {
        let db = crate::database::Database::new("sqlite::memory:").await;
        let (reader_registry, _) = Registry::<FakeFactory, _>::new(db).split();
        let client = Client::new();

        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/vm-123/doc")
            .body(Body::empty())
            .expect("request");

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
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

        let client = Client::new();
        let request = Request::builder()
            .method(Method::GET)
            .uri("/vm/vm-123/doc")
            .body(Body::empty())
            .expect("request");

        let mut request = request;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&bearer_token_for_vm_ids(&["vm-999"], TEST_PROXY_JWT_SECRET))
                .expect("valid authorization header"),
        );

        let response = proxy_vm_request(
            &reader_registry,
            &client,
            TEST_PROXY_JWT_SECRET,
            request,
            OPENCODE_UPSTREAM_PORT,
            ProxyRuntimeSettings::from_timeout_millis(TEST_TIMEOUT_MILLIS),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
