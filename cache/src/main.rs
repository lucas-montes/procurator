// cache_service/src/nix_serve.rs
use axum::{
    Router,
    routing::get,
    extract::{Path, State},
    http::StatusCode,
    response::{Response, IntoResponse},
    body::Body,
};
use tokio::process::Command;
use tokio_util::io::ReaderStream;
use std::sync::Arc;


pub struct NixServeState {
    store_dir: String,
    secret_key: Option<String>,
}

impl NixServeState {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let store_dir = std::env::var("NIX_STORE_DIR")
            .unwrap_or_else(|_| "/nix/store".to_string());

        tracing::info!("Using store directory: {}", store_dir);

        let secret_key = std::env::var("NIX_SECRET_KEY_FILE")
            .ok()
            .and_then(|path| {
                tracing::info!("Loading secret key from: {}", path);
                std::fs::read_to_string(path).ok()
            })
            .map(|s| s.trim().to_string());

        if secret_key.is_some() {
            tracing::info!("Secret key loaded successfully");
        } else {
            tracing::warn!("No secret key configured - cache will not sign packages");
        }

        Ok(Self { store_dir, secret_key })
    }
}

pub fn router() -> Router {
    let state = NixServeState::new().expect("Failed to initialize nix-serve state");

    Router::new()
        .route("/nix-cache-info", get(nix_cache_info))
        .route("/{hash_narinfo}", get(narinfo))
        .route("/nar/{nar_file}", get(nar_handler))
        .route("/log/{*store_path}", get(log))
        .with_state(Arc::new(state))
}

async fn nix_cache_info(
    State(state): State<Arc<NixServeState>>,
) -> impl IntoResponse {
    let response = format!(
        "StoreDir: {}\nWantMassQuery: 1\nPriority: 30\n",
        state.store_dir
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(Body::from(response))
        .unwrap()
}

async fn narinfo(
    State(state): State<Arc<NixServeState>>,
    Path(hash_narinfo): Path<String>,
) -> Result<Response, StatusCode> {
    // Extract hash part from "hash.narinfo"
    let hash_part = hash_narinfo.strip_suffix(".narinfo")
        .ok_or(StatusCode::BAD_REQUEST)?;

    tracing::debug!("Requested narinfo for hash: {}", hash_part);

    // Validate hash part (only lowercase hex)
    if !hash_part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        tracing::warn!("Invalid hash part format: {}", hash_part);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Query store path from hash part
    let output = Command::new("nix-store")
        .args(["--query", "--hash"])
        .arg(format!("{}/{}", state.store_dir, hash_part))
        .output()
        .await
        .map_err(|e| {
            tracing::error!("Failed to query nix-store: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !output.status.success() {
        tracing::debug!("Store path not found for hash: {}", hash_part);
        return Err(StatusCode::NOT_FOUND);
    }

    let store_path = format!("{}/{}", state.store_dir, hash_part);
    tracing::debug!("Found store path: {}", store_path);

    // Query path info
    let path_info = query_path_info(&store_path).await
        .map_err(|e| {
            tracing::error!("Failed to query path info: {}", e);
            StatusCode::NOT_FOUND
        })?;

    // Extract sha256 hash from nar_hash (format: "sha256:base32hash")
    let nar_hash_parts: Vec<&str> = path_info.nar_hash.split(':').collect();
    if nar_hash_parts.len() != 2 || nar_hash_parts[0] != "sha256" {
        tracing::error!("Invalid nar_hash format: {}", path_info.nar_hash);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let nar_hash2 = nar_hash_parts[1];

    if nar_hash2.len() != 52 {
        tracing::error!("Invalid nar_hash length: {}", nar_hash2.len());
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Build narinfo response
    let mut response = format!(
        "StorePath: {}\n\
         URL: nar/{}-{}.nar\n\
         Compression: none\n\
         NarHash: {}\n\
         NarSize: {}\n",
        store_path,
        hash_part,
        nar_hash2,
        path_info.nar_hash,
        path_info.nar_size
    );

    // Add references
    if !path_info.references.is_empty() {
        let refs: Vec<String> = path_info.references
            .iter()
            .map(|r| strip_path(r))
            .collect();
        response.push_str(&format!("References: {}\n", refs.join(" ")));
    }

    // Add deriver
    if let Some(deriver) = path_info.deriver {
        response.push_str(&format!("Deriver: {}\n", strip_path(&deriver)));
    }

    // Add signature
    if let Some(ref secret_key) = state.secret_key {
        let fingerprint = fingerprint_path(
            &store_path,
            &path_info.nar_hash,
            path_info.nar_size,
            &path_info.references,
        );
        tracing::debug!("Fingerprint to sign: {}", fingerprint);
        let signature = sign_string(secret_key, &fingerprint)
            .map_err(|e| {
                tracing::error!("Failed to sign: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        tracing::debug!("Generated signature: {}", signature);
        response.push_str(&format!("Sig: {}\n", signature));
    } else if !path_info.signatures.is_empty() {
        for sig in &path_info.signatures {
            response.push_str(&format!("Sig: {}\n", sig));
        }
    }

    tracing::info!("Serving narinfo for {}", hash_part);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/x-nix-narinfo")
        .header("Content-Length", response.len())
        .body(Body::from(response))
        .unwrap())
}

// Combined handler for both new and legacy NAR formats
async fn nar_handler(
    State(state): State<Arc<NixServeState>>,
    Path(nar_file): Path<String>,
) -> Result<Response, StatusCode> {
    tracing::debug!("NAR request for file: {}", nar_file);

    // Parse filename: either "hash_part-nar_hash.nar" or "hash_part.nar" (legacy)
    let filename = nar_file.strip_suffix(".nar")
        .ok_or_else(|| {
            tracing::warn!("Invalid NAR filename: {}", nar_file);
            StatusCode::BAD_REQUEST
        })?;

    let (hash_part, expected_nar_hash) = if let Some(dash_pos) = filename.rfind('-') {
        // New format: "hash_part-nar_hash.nar"
        let hash_part = &filename[..dash_pos];
        let nar_hash = &filename[dash_pos + 1..];
        tracing::debug!("New format NAR: hash={}, nar_hash={}", hash_part, nar_hash);
        (hash_part.to_string(), Some(nar_hash.to_string()))
    } else {
        // Legacy format: "hash_part.nar"
        tracing::debug!("Legacy format NAR: hash={}", filename);
        (filename.to_string(), None)
    };

    // Validate hash part
    if !hash_part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        tracing::warn!("Invalid hash part: {}", hash_part);
        return Err(StatusCode::BAD_REQUEST);
    }

    let store_path = format!("{}/{}", state.store_dir, hash_part);

    // Query path info
    let path_info = query_path_info(&store_path).await
        .map_err(|e| {
            tracing::error!("Failed to query path info for {}: {}", store_path, e);
            StatusCode::NOT_FOUND
        })?;

    // Verify NAR hash if provided (new format)
    if let Some(expected) = expected_nar_hash {
        if path_info.nar_hash != format!("sha256:{}", expected) {
            tracing::warn!("NAR hash mismatch: expected sha256:{}, got {}",
                expected, path_info.nar_hash);
            return Err(StatusCode::NOT_FOUND);
        }
    }

    tracing::info!("Streaming NAR for {}", store_path);

    // Stream NAR file
    let child = Command::new("nix")
        .args([
            "--extra-experimental-features", "nix-command",
            "store", "dump-path",
            "--",
            &store_path,
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            tracing::error!("Failed to spawn nix command: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let stdout = child.stdout.ok_or_else(|| {
        tracing::error!("Failed to capture stdout");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let stream = ReaderStream::new(stdout);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/x-nix-archive")
        .header("Content-Length", path_info.nar_size)
        .body(body)
        .unwrap())
}

async fn log(
    State(state): State<Arc<NixServeState>>,
    Path(store_path_suffix): Path<String>,
) -> Result<Response, StatusCode> {
    let store_path = format!("{}/{}", state.store_dir, store_path_suffix);
    tracing::debug!("Requesting log for: {}", store_path);

    let child = Command::new("nix")
        .args([
            "--extra-experimental-features", "nix-command",
            "log",
            &store_path,
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            tracing::error!("Failed to spawn nix log command: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let stdout = child.stdout.ok_or_else(|| {
        tracing::error!("Failed to capture stdout");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let stream = ReaderStream::new(stdout);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .body(body)
        .unwrap())
}

// Helper structs and functions

#[derive(Debug)]
struct PathInfo {
    nar_hash: String,
    nar_size: u64,
    references: Vec<String>,
    deriver: Option<String>,
    signatures: Vec<String>,
}

async fn query_path_info(store_path: &str) -> Result<PathInfo, Box<dyn std::error::Error>> {
    tracing::debug!("Querying path info for: {}", store_path);

    // Query using nix-store
    let output = Command::new("nix-store")
        .args(["--query", "--deriver", "--hash", "--size", "--references", store_path])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("nix-store query failed: {}", stderr);
        return Err("Failed to query path info".into());
    }

    let output_str = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = output_str.lines().collect();

    // Parse output (simplified - real implementation needs proper parsing)
    let nar_hash = lines.get(0).unwrap_or(&"").to_string();
    let nar_size: u64 = lines.get(1).unwrap_or(&"0").parse().unwrap_or(0);
    let references: Vec<String> = lines.iter().skip(2).map(|s| s.to_string()).collect();

    tracing::debug!("Path info: hash={}, size={}, refs={}", nar_hash, nar_size, references.len());

    // Query signatures
    let sigs_output = Command::new("nix-store")
        .args(["--query", "--sigs", store_path])
        .output()
        .await?;

    let signatures: Vec<String> = if sigs_output.status.success() {
        String::from_utf8(sigs_output.stdout)?
            .lines()
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };

    Ok(PathInfo {
        nar_hash,
        nar_size,
        references,
        deriver: None, // TODO: parse deriver
        signatures,
    })
}

fn strip_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn fingerprint_path(
    store_path: &str,
    nar_hash: &str,
    nar_size: u64,
    references: &[String],
) -> String {
    // Format: "1;/nix/store/path;sha256:hash;size;ref1,ref2,..."
    let refs = references.join(",");
    format!("1;{};{};{};{}", store_path, nar_hash, nar_size, refs)
}

fn sign_string(secret_key: &str, message: &str) -> Result<String, Box<dyn std::error::Error>> {
    use ed25519_dalek::{Signer, SigningKey};
    use base64::{Engine as _, engine::general_purpose};

    // Parse secret key (format: "keyname:base64key")
    let parts: Vec<&str> = secret_key.split(':').collect();
    if parts.len() != 2 {
        return Err("Invalid secret key format".into());
    }

    let key_name = parts[0];
    let key_bytes = general_purpose::STANDARD.decode(parts[1])?;

    if key_bytes.len() != 32 {
        return Err("Invalid key length".into());
    }

    let signing_key = SigningKey::from_bytes(&key_bytes.try_into().unwrap());
    let signature = signing_key.sign(message.as_bytes());

    let sig_b64 = general_purpose::STANDARD.encode(signature.to_bytes());

    Ok(format!("{}:{}", key_name, sig_b64))
}

// Main server
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    tracing::info!("Starting Procurator cache service");

    let app = router();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081").await?;
    tracing::info!("Cache service listening on http://0.0.0.0:8081");
    tracing::info!("Endpoints:");
    tracing::info!("  GET  /nix-cache-info");
    tracing::info!("  GET  /:hash.narinfo");
    tracing::info!("  GET  /nar/:file.nar");
    tracing::info!("  GET  /log/*path");

    axum::serve(listener, app).await?;

    Ok(())
}
