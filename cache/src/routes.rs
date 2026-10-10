//! HTTP routes, generic over any `StorageBackend` (no `dyn`).
//! Why: handlers only know the trait; `LocalDisk` vs future `S3` is chosen in `main`.

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Extension, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::io::ReaderStream;

use crate::config::Config;
use crate::nix::{
    Narinfo, base_name, check_hash, fingerprint, parse_narinfo, query_path_info, render_narinfo,
    resolve_store_path, sign_with, sri_to_narinfo,
};
use crate::storage::{StorageBackend, validate_hash};
use crate::tls::PeerIdentity;

/// Shared state, monomorphized per backend. Why: static dispatch, no `dyn`.
#[derive(Debug, Clone)]
pub struct AppState<S> {
    /// Runtime config.
    pub config: Arc<Config>,
    /// Chosen storage backend.
    pub store: S,
}

/// Build the router for any backend.
pub fn router<S>(state: AppState<S>) -> Router
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    Router::new()
        .route("/nix-cache-info", get(nix_cache_info::<S>))
        .route("/healthz", get(healthz::<S>))
        .route(
            // Why: axum allows one param per segment, so capture `hash.narinfo` whole and strip suffix in handler.
            "/{narinfo}",
            get(get_narinfo::<S>)
                .put(put_narinfo::<S>)
                .head(head_narinfo::<S>),
        )
        .route(
            "/nar/{file}",
            get(get_nar::<S>).put(put_nar::<S>).head(head_nar::<S>),
        )
        .route("/", axum::routing::post(mass_query::<S>))
        .route("/mass-query", axum::routing::post(mass_query::<S>))
        .route("/log/{*path}", get(get_log::<S>))
        // Why: labels loopback peers for the bypass; skips when mTLS already did.
        .layer(middleware::from_fn(peer_loopback_mw))
        .with_state(state)
}

/// Mark loopback peers on the plain-HTTP path (no TLS acceptor there).
async fn peer_loopback_mw(mut req: Request, next: Next) -> Response {
    // Why: axum::serve has no per-connection hook, so label here instead.
    let is_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|c| c.0.ip().is_loopback());
    if req.extensions().get::<PeerIdentity>().is_none() {
        let mut id = PeerIdentity::anonymous();
        id.is_loopback = is_loopback;
        req.extensions_mut().insert(id);
    }
    next.run(req).await
}

/// Who is calling? Why: mTLS acceptor injects identity; plain HTTP stays anonymous.
fn peer_of(peer: Option<Extension<PeerIdentity>>) -> PeerIdentity {
    peer.map_or_else(PeerIdentity::anonymous, |Extension(p)| p)
}

/// Writes need a verified client unless open; loopback bypass covers co-located CI.
/// Why: open PUT on a public bind is refused at startup, so open here is safe.
fn require_writer(config: &Config, peer: &PeerIdentity) -> Result<(), StatusCode> {
    if config.writes_open() {
        return Ok(());
    }
    if config.access.loopback_bypass && peer.is_loopback {
        return Ok(());
    }
    if !peer.verified {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if peer.matches_allowlist(&config.tls.writer_allowlist) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Reads need a verified client only in private mode; loopback bypass applies too.
fn require_reader(config: &Config, peer: &PeerIdentity) -> Result<(), StatusCode> {
    if !config.access.reads_require_auth {
        return Ok(());
    }
    if config.access.loopback_bypass && peer.is_loopback {
        return Ok(());
    }
    if peer.verified {
        return Ok(());
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn nix_cache_info<S>(State(state): State<AppState<S>>) -> impl IntoResponse
where
    S: StorageBackend + Send + Sync,
{
    // Why: `WantMassQuery: 1` is true — we implement `POST /` mass-query below.
    let body = format!(
        "StoreDir: {}\nWantMassQuery: 1\nPriority: {}\n",
        state.config.store_dir, state.config.priority
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn healthz<S>(State(state): State<AppState<S>>) -> impl IntoResponse
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let usage = state.store.usage_bytes().await.map_err(|e| e.to_string());
    match usage {
        Ok(used) => format!("ok usage_bytes={used}").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Serve narinfo: disk first (uploads), then local `/nix/store` proxy.
/// Why: uploaded closures survive GC on the server; proxy covers the rest.
async fn get_narinfo<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(hash): Path<String>,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_reader(&state.config, &peer)?;
    let hash = hash.strip_suffix(".narinfo").unwrap_or(&hash);
    validate_hash(hash).map_err(|_| StatusCode::BAD_REQUEST)?;
    let body = narinfo_for_hash(&state, hash).await?;
    Ok(narinfo_response(&body))
}

/// Shared lookup: disk first (uploads), then local `/nix/store` proxy.
/// Why: one code path serves `GET`, `HEAD`, and mass-query consistently.
async fn narinfo_for_hash<S>(state: &AppState<S>, hash: &str) -> Result<String, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    // Why: disk first so `nix copy --to` results are immediately visible.
    if let Ok(Some(body)) = state.store.get_narinfo(hash).await {
        return Ok(body);
    }
    let store_path = resolve_store_path(&state.config.store_dir, hash)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let info = query_path_info(&store_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let nar_hash = sri_to_narinfo(&info.nar_hash).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let refs: Vec<String> = info
        .references
        .iter()
        .map(|r| base_name(r).to_string())
        .collect();
    let deriver = info.deriver.as_deref().map(base_name);
    let ca = info.ca.as_ref().and_then(|v| v.as_str());
    // Why: keep upstream sigs, then add ours so clients trusting either key succeed.
    let mut sigs = info.signatures.clone();
    for key in &state.config.sign_keys {
        sigs.push(sign_with(
            &key.name,
            &key.seed,
            &fingerprint(&store_path, &nar_hash, info.nar_size, &refs),
        ));
    }
    Ok(render_narinfo(&Narinfo {
        store_path: &store_path,
        file_hash: &nar_hash,
        file_size: info.nar_size,
        nar_hash: &nar_hash,
        nar_size: info.nar_size,
        refs: &refs,
        deriver,
        sigs: &sigs,
        ca,
    }))
}

fn narinfo_response(body: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/x-nix-narinfo")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn head_narinfo<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(hash): Path<String>,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    // Why: `HEAD` lets Nix probe existence without downloading.
    let resp = get_narinfo::<S>(State(state), peer, Path(hash)).await?;
    Ok(Response::builder()
        .status(resp.status())
        .header("Content-Type", "text/x-nix-narinfo")
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

/// Upload a narinfo. Why: this is what `nix copy --to https://cache` calls.
async fn put_narinfo<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(hash): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_writer(&state.config, &peer)?;
    let hash = hash.strip_suffix(".narinfo").unwrap_or(&hash);
    check_hash(hash).map_err(|_| StatusCode::BAD_REQUEST)?;
    if body.len() > 1_048_576 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let parsed = parse_narinfo(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    // Why: bind route hash to `StorePath` so uploads cannot shadow other paths.
    let expected = parsed
        .store_path
        .strip_prefix(&format!("{}/", state.config.store_dir))
        .unwrap_or(&parsed.store_path);
    if !expected.starts_with(&format!("{hash}-")) {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Why: require a signature (uploaded or server-added) so workers trust content.
    let mut final_body = body;
    if !final_body.contains("Sig:") {
        let Some(key) = state.config.sign_keys.first() else {
            return Err(StatusCode::BAD_REQUEST);
        };
        let refs: Vec<String> = parsed.references.clone();
        let sig = sign_with(
            &key.name,
            &key.seed,
            &fingerprint(&parsed.store_path, &parsed.nar_hash, parsed.nar_size, &refs),
        );
        let _ = writeln!(final_body, "Sig: {sig}");
    }
    let _ = headers;
    state
        .store
        .put_narinfo(hash, final_body)
        .await
        .map_err(|e| map_storage_error(&e.to_string()))?;
    Ok(StatusCode::CREATED.into_response())
}

/// Serve a NAR: disk first, else stream `nix store dump-path`.
async fn get_nar<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(file): Path<String>,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_reader(&state.config, &peer)?;
    if let Ok(Some((size, path))) = state.store.get_nar(&file).await {
        // Why: uploaded NARs stream from disk; no Nix subprocess needed.
        let stream = tokio::fs::File::open(&path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let body = Body::from_stream(ReaderStream::new(stream));
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/x-nix-nar")
            .header("Content-Length", size)
            .body(body)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()));
    }
    stream_nar_from_store(&state.config.store_dir, &file).await
}

async fn head_nar<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(file): Path<String>,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_reader(&state.config, &peer)?;
    if let Ok(Some((size, _))) = state.store.get_nar(&file).await {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/x-nix-nar")
            .header("Content-Length", size)
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()));
    }
    // Why: fall back to store metadata so HEAD works for proxied paths too.
    match stream_nar_from_store(&state.config.store_dir, &file).await {
        Ok(resp) => Ok(Response::builder()
            .status(resp.status())
            .header("Content-Type", "application/x-nix-nar")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())),
        Err(status) => Err(status),
    }
}

/// Upload a NAR file. Why: pairs with `PUT *.narinfo` for `nix copy --to`.
async fn put_nar<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(file): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_writer(&state.config, &peer)?;
    if body.len() > 2_147_483_648 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    // Why: stage under storage dir so rename stays on one filesystem (atomic).
    let tmp = state.config.storage_dir.join(format!(".upload-{file}.tmp"));
    tokio::fs::write(&tmp, &body)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .store
        .put_nar(&file, &tmp)
        .await
        .map_err(|e| map_storage_error(&e.to_string()))?;
    Ok(StatusCode::CREATED.into_response())
}

/// Mass query: `POST /` with JSON store paths → narinfo map.
/// Why: lets clients check many paths in one round trip (`WantMassQuery: 1`).
async fn mass_query<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Query(query): Query<HashMap<String, String>>,
    body: String,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::fmt::Display + Send + Sync,
{
    let peer = peer_of(peer);
    require_reader(&state.config, &peer)?;
    let _ = query;
    // Why: accept both `{"storePaths":[...]}` and `{"paths":[...]}` shapes.
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let paths: Vec<String> = json
        .get("storePaths")
        .or_else(|| json.get("paths"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .take(1000)
                .collect()
        })
        .unwrap_or_default();
    let mut out = serde_json::Map::new();
    for path in paths {
        let hash = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .split('-')
            .next()
            .unwrap_or("")
            .to_string();
        if hash.len() != 32 {
            continue;
        }
        // Why: same disk+proxy path as `GET`, so mass-query sees everything.
        if let Ok(narinfo) = narinfo_for_hash(&state, &hash).await {
            out.insert(path, serde_json::Value::String(narinfo));
        }
    }
    Ok(axum::Json(serde_json::Value::Object(out)).into_response())
}

async fn get_log<S>(
    State(state): State<AppState<S>>,
    peer: Option<Extension<PeerIdentity>>,
    Path(path): Path<String>,
) -> Result<Response, StatusCode>
where
    S: StorageBackend + Send + Sync,
{
    let peer = peer_of(peer);
    require_reader(&state.config, &peer)?;
    // Why: build logs help CI debugging; streams `nix log` output.
    let store_path = format!("{}/{}", state.config.store_dir, path);
    let mut child = tokio::process::Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command",
            "log",
            &store_path,
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(Body::from_stream(ReaderStream::new(stdout)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

/// Stream an uncompressed NAR for `nar/<storehash>-<filehash>.nar`.
/// Why: bare `<filehash>.nar` names serve from disk only (uploads) — the proxy
/// has no filehash→path index, so those 404 here with a hint to upload first.
async fn stream_nar_from_store(store_dir: &str, file: &str) -> Result<Response, StatusCode> {
    let base = file.strip_suffix(".nar").ok_or(StatusCode::BAD_REQUEST)?;
    // Why: first segment is the 32-char store hash; the rest is the file hash.
    let hash_part = base.split('-').next().unwrap_or(base);
    check_hash(hash_part).map_err(|_| {
        tracing::warn!("NAR {file} has no store-hash prefix and is not on disk");
        StatusCode::NOT_FOUND
    })?;
    let store_path = resolve_store_path(store_dir, hash_part)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let info = query_path_info(&store_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let mut child = tokio::process::Command::new("nix")
        .args([
            "--extra-experimental-features",
            "nix-command",
            "store",
            "dump-path",
            "--",
            &store_path,
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stdout = child
        .stdout
        .take()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/x-nix-nar")
        .header("Content-Length", info.nar_size)
        .body(Body::from_stream(ReaderStream::new(stdout)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

fn map_storage_error(msg: &str) -> StatusCode {
    // Why: quota and bad names are client errors; the rest are server faults.
    if msg.contains("quota") {
        StatusCode::INSUFFICIENT_STORAGE
    } else if msg.contains("invalid") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
