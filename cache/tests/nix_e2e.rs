//! End-to-end test with a real `nix` client against the real adapters
//! (disk blobs, `SQLite` index, TCP listener).
//!
//! Ignored by default because it needs `nix` on PATH and builds a derivation:
//!   cargo test -p cache --test `nix_e2e` -- --ignored

use std::path::Path;
use std::process::Output;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use cache::adapters::fs::FsBlobStore;
use cache::adapters::http::router;
use cache::adapters::sqlite::SqliteIndex;
use cache::adapters::system::SystemEnv;
use cache::application::{NewToken, Service};
use cache::domain::access::Principal;
use cache::domain::cache::{CacheName, CacheSpec, Visibility};
use cache::domain::nix::SigningKey;
use tokio::process::Command;

/// A derivation whose output references another store path, so the pull
/// exercises the reference list inside the signed fingerprint.
const DERIVATION: &str = r#"derivation {
  name = "procurator-cache-e2e";
  system = builtins.currentSystem;
  builder = "/bin/sh";
  args = [ "-c" "echo ${builtins.toFile "dep" "dependency"} > $out" ];
}"#;

/// Run `nix` with flakes-era commands enabled and narinfo caching off.
/// Why: the client caches narinfos per URL; a reused port must not hit them.
async fn nix(args: &[&str]) -> Output {
    Command::new("nix")
        .args(["--extra-experimental-features", "nix-command"])
        .args(["--option", "narinfo-cache-positive-ttl", "0"])
        .args(["--option", "narinfo-cache-negative-ttl", "0"])
        .args(args)
        .output()
        .await
        .expect("run nix")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `nix copy` the path out of the cache into an empty store under `dest`.
async fn pull(
    url: &str,
    dest: &Path,
    netrc: Option<&Path>,
    trusted_key: &str,
    path: &str,
) -> Output {
    let dest = dest.display().to_string();
    let netrc = netrc.map(|p| p.display().to_string());
    let mut args = vec!["copy", "--from", url, "--to", &dest];
    if let Some(netrc) = &netrc {
        args.extend(["--option", "netrc-file", netrc]);
    }
    args.extend(["--option", "trusted-public-keys", trusted_key, path]);
    nix(&args).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs nix on PATH"]
async fn nix_client_pushes_and_pulls_through_a_private_cache() {
    let dir = tempfile::tempdir().unwrap();
    let key = SigningKey::parse(&format!("e2e-1:{}", STANDARD.encode([42_u8; 32]))).unwrap();
    let public_key = key.public_key();
    let spec = CacheSpec::new(
        CacheName::parse("private").unwrap(),
        Visibility::Private,
        String::from("/nix/store"),
        30,
        0,
        None,
        key,
    );
    let service = Arc::new(Service::new(
        FsBlobStore::open(dir.path()).unwrap(),
        SqliteIndex::open(&dir.path().join("index.sqlite"))
            .await
            .unwrap(),
        SystemEnv,
        vec![spec],
        None,
        86_400,
    ));
    // Why: minting in-process shows the use cases work without HTTP.
    let (_, secret) = service
        .create_token(
            &Principal::System,
            NewToken::new("e2e", false, vec![("private", "write")]),
        )
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/private", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router(service)).await });

    let netrc = dir.path().join("netrc");
    std::fs::write(
        &netrc,
        format!("machine 127.0.0.1 password {}\n", secret.as_str()),
    )
    .unwrap();
    let netrc_arg = netrc.display().to_string();

    let built = nix(&[
        "build",
        "--no-link",
        "--print-out-paths",
        "--impure",
        "--expr",
        DERIVATION,
    ])
    .await;
    assert!(built.status.success(), "build: {}", stderr(&built));
    let path = String::from(String::from_utf8_lossy(&built.stdout).trim());

    // Pushing without credentials is refused; with the token it works.
    let refused = nix(&["copy", "--to", &url, &path]).await;
    assert!(!refused.status.success(), "anonymous push must fail");
    let pushed = nix(&[
        "copy",
        "--to",
        &url,
        "--option",
        "netrc-file",
        &netrc_arg,
        &path,
    ])
    .await;
    assert!(pushed.status.success(), "push: {}", stderr(&pushed));

    // Pulling into an empty store: the locally built path carried no
    // signature, so success proves the cache's own signature verified.
    let dest = dir.path().join("dest");
    let pulled = pull(&url, &dest, Some(&netrc), &public_key, &path).await;
    assert!(pulled.status.success(), "pull: {}", stderr(&pulled));
    let copied = std::fs::read_to_string(dest.join(path.trim_start_matches('/'))).unwrap();
    assert!(copied.starts_with("/nix/store/") && copied.trim_end().ends_with("-dep"));

    // A client trusting a different key rejects the path.
    let other_key = SigningKey::parse(&format!("e2e-1:{}", STANDARD.encode([1_u8; 32]))).unwrap();
    let untrusted = pull(
        &url,
        &dir.path().join("untrusted"),
        Some(&netrc),
        &other_key.public_key(),
        &path,
    )
    .await;
    assert!(!untrusted.status.success(), "wrong key must be rejected");
    assert!(
        stderr(&untrusted).contains("signature"),
        "{}",
        stderr(&untrusted)
    );

    // A client without the token cannot pull from a private cache.
    let anonymous = pull(
        &url,
        &dir.path().join("anonymous"),
        None,
        &public_key,
        &path,
    )
    .await;
    assert!(!anonymous.status.success(), "anonymous pull must fail");

    server.abort();
}
