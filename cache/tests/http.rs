//! HTTP adapter tests: the router driven in-process, no sockets.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, WWW_AUTHENTICATE};
use axum::http::{Method, Request, StatusCode};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cache::adapters::http::router;
use cache::adapters::memory::{MemoryBlobStore, MemoryIndex};
use cache::adapters::system::SystemEnv;
use cache::application::Service;
use cache::domain::cache::{CacheName, CacheSpec, Visibility};
use cache::domain::digest::Digest;
use cache::domain::nix::SigningKey;
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

const ROOT: &str = "root-secret";

/// A NAR big enough to arrive in several chunks.
static NAR: [u8; 256 * 1024] = [7; 256 * 1024];

fn spec(name: &str, visibility: Visibility, seed: u8) -> CacheSpec {
    let key = SigningKey::parse(&format!("{name}-key:{}", STANDARD.encode([seed; 32]))).unwrap();
    CacheSpec::new(
        CacheName::parse(name).unwrap(),
        visibility,
        String::from("/nix/store"),
        30,
        0,
        None,
        key,
    )
}

/// A router over in-memory adapters with caches `pub` and `priv`.
fn app() -> Router {
    router(Arc::new(Service::new(
        MemoryBlobStore::new(),
        MemoryIndex::new(),
        SystemEnv,
        vec![
            spec("pub", Visibility::Public, 1),
            spec("priv", Visibility::Private, 2),
        ],
        Some(Digest::of(ROOT.as_bytes())),
        86_400,
    )))
}

/// One response, fully read.
struct Reply {
    status: StatusCode,
    challenge: bool,
    length: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: impl Into<Body>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if path.starts_with("/_api") {
        request = request.header("content-type", "application/json");
    }
    let response = app
        .clone()
        .oneshot(request.body(body.into()).unwrap())
        .await
        .unwrap();
    Reply {
        status: response.status(),
        challenge: response.headers().contains_key(WWW_AUTHENTICATE),
        length: response
            .headers()
            .get(CONTENT_LENGTH)
            .map(|v| String::from(v.to_str().unwrap())),
        body: response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    }
}

async fn get(app: &Router, path: &str, token: Option<&str>) -> Reply {
    send(app, Method::GET, path, token, Body::empty()).await
}

/// Mint a token through the admin API and return its secret.
async fn mint(app: &Router, grants: &[(&str, &str)]) -> String {
    let grants: Vec<_> = grants
        .iter()
        .map(|(cache, permission)| serde_json::json!({"cache": cache, "permission": permission}))
        .collect();
    let request = serde_json::json!({"name": "test", "grants": grants}).to_string();
    let reply = send(app, Method::POST, "/_api/v1/tokens", Some(ROOT), request).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    String::from(reply.json()["token"].as_str().unwrap())
}

fn store_hash(letter: char) -> String {
    String::from(letter).repeat(32)
}

fn nar_path(cache: &str, nar: &[u8]) -> String {
    format!("/{cache}/nar/{}.nar", Digest::of(nar).to_nix32())
}

fn narinfo_path(cache: &str, letter: char) -> String {
    format!("/{cache}/{}.narinfo", store_hash(letter))
}

fn narinfo(letter: char, nar: &[u8]) -> String {
    let hash = Digest::of(nar).to_nix32();
    format!(
        "StorePath: /nix/store/{}-pkg\nURL: nar/{hash}.nar\nCompression: none\n\
         FileHash: sha256:{hash}\nFileSize: {len}\nNarHash: sha256:{hash}\nNarSize: {len}\n\
         References: \n",
        store_hash(letter),
        len = nar.len(),
    )
}

/// Push one path with `token`, asserting both uploads succeed.
async fn push(app: &Router, cache: &str, token: &str, letter: char, nar: &'static [u8]) {
    let uploaded = send(app, Method::PUT, &nar_path(cache, nar), Some(token), nar).await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.text());
    let described = send(
        app,
        Method::PUT,
        &narinfo_path(cache, letter),
        Some(token),
        narinfo(letter, nar),
    )
    .await;
    assert_eq!(
        described.status,
        StatusCode::CREATED,
        "{}",
        described.text()
    );
}

#[tokio::test]
async fn public_cache_access_matrix() {
    let app = app();
    let writer = mint(&app, &[("pub", "write")]).await;
    let reader = mint(&app, &[("pub", "read")]).await;
    let nar = b"public-nar";

    // Anonymous write: 401 with a challenge so curl sends credentials.
    let refused = send(&app, Method::PUT, &nar_path("pub", nar), None, &nar[..]).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert!(refused.challenge);
    // Read-only token: 403.
    let forbidden = send(
        &app,
        Method::PUT,
        &nar_path("pub", nar),
        Some(&reader),
        &nar[..],
    )
    .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    // Write token: accepted.
    push(&app, "pub", &writer, 'a', nar).await;

    // Anonymous read of everything.
    let info = get(&app, "/pub/nix-cache-info", None).await;
    assert_eq!(info.status, StatusCode::OK);
    assert!(info.text().contains("StoreDir: /nix/store"));
    let stored = get(&app, &narinfo_path("pub", 'a'), None).await;
    assert_eq!(stored.status, StatusCode::OK);
    assert!(stored.text().contains("Sig: pub-key:"));
    let fetched = get(&app, &nar_path("pub", nar), None).await;
    assert_eq!(
        (fetched.status, fetched.body.as_slice()),
        (StatusCode::OK, &nar[..])
    );
    let head = send(
        &app,
        Method::HEAD,
        &nar_path("pub", nar),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.length.as_deref(), Some("10"));
    assert!(head.body.is_empty());
    let head = send(
        &app,
        Method::HEAD,
        &narinfo_path("pub", 'a'),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(head.status, StatusCode::OK);

    // Misses and a bad credential.
    assert_eq!(
        get(&app, &narinfo_path("pub", 'z'), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &nar_path("pub", b"nope"), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/pub/favicon.ico", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/pub/nix-cache-info", Some("not-a-token"))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn private_cache_access_matrix() {
    let app = app();
    let writer = mint(&app, &[("priv", "write")]).await;
    let reader = mint(&app, &[("priv", "read")]).await;
    let outsider = mint(&app, &[("pub", "write")]).await;
    let nar = b"private-nar";
    push(&app, "priv", &writer, 'a', nar).await;

    for path in [
        String::from("/priv/nix-cache-info"),
        narinfo_path("priv", 'a'),
        nar_path("priv", nar),
    ] {
        let anonymous = get(&app, &path, None).await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{path}");
        assert!(anonymous.challenge, "{path}");
        assert_eq!(
            get(&app, &path, Some(&outsider)).await.status,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_eq!(
            get(&app, &path, Some(&reader)).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    let head = send(
        &app,
        Method::HEAD,
        &nar_path("priv", nar),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(head.status, StatusCode::UNAUTHORIZED);
    let write = send(
        &app,
        Method::PUT,
        &nar_path("priv", nar),
        Some(&reader),
        &nar[..],
    )
    .await;
    assert_eq!(write.status, StatusCode::FORBIDDEN);

    // A netrc-style Basic credential works like Bearer.
    let basic = format!("Basic {}", STANDARD.encode(format!("nix:{reader}")));
    let request = Request::get(narinfo_path("priv", 'a'))
        .header(AUTHORIZATION, basic)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn caches_are_isolated() {
    let app = app();
    let both = mint(&app, &[("pub", "write"), ("priv", "write")]).await;
    let nar = b"only-in-private";
    push(&app, "priv", &both, 'a', nar).await;

    assert_eq!(
        get(&app, &narinfo_path("pub", 'a'), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &nar_path("pub", nar), None).await.status,
        StatusCode::NOT_FOUND
    );
    // A narinfo cannot claim a NAR that only another cache holds.
    let claim = send(
        &app,
        Method::PUT,
        &narinfo_path("pub", 'a'),
        Some(&both),
        narinfo('a', nar),
    )
    .await;
    assert_eq!(claim.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        get(&app, "/nope/nix-cache-info", None).await.status,
        StatusCode::NOT_FOUND
    );

    push(&app, "pub", &both, 'a', nar).await;
    let public = get(&app, &narinfo_path("pub", 'a'), None).await;
    let private = get(&app, &narinfo_path("priv", 'a'), Some(&both)).await;
    assert!(public.text().contains("Sig: pub-key:") && !public.text().contains("priv-key"));
    assert!(private.text().contains("Sig: priv-key:") && !private.text().contains("pub-key"));
}

#[tokio::test]
async fn nar_hash_mismatch_rejected() {
    let app = app();
    let writer = mint(&app, &[("pub", "write")]).await;
    let path = nar_path("pub", b"what-the-name-claims");
    let lie = send(&app, Method::PUT, &path, Some(&writer), "different bytes").await;
    assert_eq!(lie.status, StatusCode::BAD_REQUEST);
    assert_eq!(get(&app, &path, None).await.status, StatusCode::NOT_FOUND);

    let bad_name = send(&app, Method::PUT, "/pub/nar/evil.nar", Some(&writer), "x").await;
    assert_eq!(bad_name.status, StatusCode::BAD_REQUEST);
    let bad_info = send(
        &app,
        Method::PUT,
        &narinfo_path("pub", 'a'),
        Some(&writer),
        "junk",
    )
    .await;
    assert_eq!(bad_info.status, StatusCode::BAD_REQUEST);
    let huge = "x".repeat((1 << 20) + 1);
    let too_big = send(
        &app,
        Method::PUT,
        &narinfo_path("pub", 'a'),
        Some(&writer),
        huge,
    )
    .await;
    assert_eq!(too_big.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn concurrent_upload_is_intact() {
    let app = app();
    let writer = mint(&app, &[("pub", "write")]).await;
    let nar: &'static [u8] = &NAR;
    let path = nar_path("pub", nar);
    let uploads = (0..8).map(|_| send(&app, Method::PUT, &path, Some(&writer), nar));
    for reply in futures::future::join_all(uploads).await {
        assert_eq!(reply.status, StatusCode::CREATED);
    }
    let fetched = get(&app, &path, None).await;
    assert_eq!(Digest::of(&fetched.body), Digest::of(nar));
}

#[tokio::test]
async fn token_admin_api() {
    let app = app();
    // Only an admin reaches the API.
    let anonymous = get(&app, "/_api/v1/tokens", None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let plain = mint(&app, &[("pub", "write")]).await;
    assert_eq!(
        get(&app, "/_api/v1/tokens", Some(&plain)).await.status,
        StatusCode::FORBIDDEN
    );
    let create = serde_json::json!({"name": "sneaky", "admin": true}).to_string();
    let refused = send(&app, Method::POST, "/_api/v1/tokens", Some(&plain), create).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    // Bad requests are refused.
    for bad in [
        serde_json::json!({"name": "x", "grants": [{"cache": "nope", "permission": "read"}]}),
        serde_json::json!({"name": "x", "grants": [{"cache": "pub", "permission": "own"}]}),
        serde_json::json!({"name": ""}),
    ] {
        let reply = send(
            &app,
            Method::POST,
            "/_api/v1/tokens",
            Some(ROOT),
            bad.to_string(),
        )
        .await;
        assert!(reply.status.is_client_error(), "{bad}");
    }

    // Listing never reveals secrets.
    let listed = get(&app, "/_api/v1/tokens", Some(ROOT)).await;
    assert_eq!(listed.status, StatusCode::OK);
    let tokens = listed.json()["tokens"].as_array().unwrap().clone();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["grants"][0]["cache"], "pub");
    assert!(tokens[0].get("token").is_none());
    assert!(!listed.text().contains(&plain));

    // Revocation takes effect immediately and is not repeatable.
    let id = tokens[0]["id"].as_str().unwrap();
    let path = format!("/_api/v1/tokens/{id}");
    assert_eq!(
        send(&app, Method::DELETE, &path, Some(&plain), Body::empty())
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, Method::DELETE, &path, Some(ROOT), Body::empty())
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, Method::DELETE, &path, Some(ROOT), Body::empty())
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let nar = b"after-revocation";
    let write = send(
        &app,
        Method::PUT,
        &nar_path("pub", nar),
        Some(&plain),
        &nar[..],
    )
    .await;
    assert_eq!(write.status, StatusCode::UNAUTHORIZED);
    let listed = get(&app, "/_api/v1/tokens", Some(ROOT)).await;
    assert!(listed.json()["tokens"][0]["revoked_at"].is_i64());
}

#[tokio::test]
async fn health_readiness_and_metrics() {
    let app = app();
    assert_eq!(get(&app, "/healthz", None).await.text(), "ok\n");
    assert_eq!(get(&app, "/readyz", None).await.status, StatusCode::OK);
    // Probes sit outside the auth layer: a bad token never fails them.
    assert_eq!(
        get(&app, "/healthz", Some("garbage")).await.status,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/readyz", Some("garbage")).await.status,
        StatusCode::OK
    );
    // Unmatched paths are 404 before any credential check.
    assert_eq!(
        get(&app, "/a/b/c/d", Some("garbage")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/metrics", None).await.status,
        StatusCode::UNAUTHORIZED
    );

    let writer = mint(&app, &[("priv", "write")]).await;
    push(&app, "priv", &writer, 'a', b"12345").await;
    assert_eq!(
        get(&app, "/metrics", Some(&writer)).await.status,
        StatusCode::FORBIDDEN
    );
    let metrics = get(&app, "/metrics", Some(ROOT)).await;
    assert!(
        metrics
            .text()
            .contains("procurator_cache_bytes{cache=\"priv\"} 5")
    );
    assert!(
        metrics
            .text()
            .contains("procurator_cache_paths{cache=\"pub\"} 0")
    );
}
