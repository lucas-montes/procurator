//! Use-case tests against in-memory adapters: no HTTP, disk, or `SQLite`.

use std::io;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures::stream;
use tokio::io::AsyncReadExt as _;

use super::{NewToken, Service};
use crate::adapters::memory::{MemoryBlobStore, MemoryIndex};
use crate::application::ServiceError;
use crate::application::ports::{BlobStore, Clock, Entropy};
use crate::domain::access::{Grant, Permission, Principal};
use crate::domain::cache::{CacheName, CacheSpec, Visibility};
use crate::domain::digest::Digest;
use crate::domain::nix::SigningKey;

const ROOT_SECRET: &str = "root-secret";
const DAY: i64 = 86_400;

/// A clock tests can move and a deterministic byte source.
struct TestEnv {
    now: AtomicI64,
    next_byte: AtomicU8,
}

impl Clock for &'static TestEnv {
    fn now(&self) -> i64 {
        self.now.load(Ordering::Relaxed)
    }
}

impl Entropy for &'static TestEnv {
    fn fill(&self, buf: &mut [u8]) {
        buf.fill(self.next_byte.fetch_add(1, Ordering::Relaxed));
    }
}

fn key(name: &str, seed: u8) -> SigningKey {
    SigningKey::parse(&format!("{name}:{}", STANDARD.encode([seed; 32]))).unwrap()
}

fn spec(name: &str, visibility: Visibility, quota: u64, retention: Option<i64>) -> CacheSpec {
    CacheSpec::new(
        CacheName::parse(name).unwrap(),
        visibility,
        String::from("/nix/store"),
        30,
        quota,
        retention,
        key(&format!("{name}-key"), 1),
    )
}

/// A service with a public cache `pub` and a private cache `priv`.
fn service_with(
    pub_quota: u64,
    retention: Option<i64>,
) -> (
    Service<MemoryBlobStore, MemoryIndex, &'static TestEnv>,
    &'static TestEnv,
) {
    let env: &'static TestEnv = Box::leak(Box::new(TestEnv {
        now: AtomicI64::new(1_000_000),
        next_byte: AtomicU8::new(1),
    }));
    let service = Service::new(
        MemoryBlobStore::new(),
        MemoryIndex::new(),
        env,
        vec![
            spec("pub", Visibility::Public, pub_quota, retention),
            spec("priv", Visibility::Private, 0, None),
        ],
        Some(Digest::of(ROOT_SECRET.as_bytes())),
        DAY,
    );
    (service, env)
}

fn service() -> Service<MemoryBlobStore, MemoryIndex, &'static TestEnv> {
    service_with(0, None).0
}

fn writer(cache: &str) -> Principal {
    token(cache, Permission::Write)
}

fn token(cache: &str, permission: Permission) -> Principal {
    Principal::Token {
        admin: false,
        grants: vec![Grant::new(CacheName::parse(cache).unwrap(), permission)],
    }
}

fn body(bytes: &'static [u8]) -> impl futures::Stream<Item = io::Result<Bytes>> + Send + Unpin {
    stream::iter([Ok(Bytes::from_static(bytes))])
}

fn nar_name(bytes: &[u8]) -> String {
    format!("{}.nar", Digest::of(bytes).to_nix32())
}

/// A store hash made of one repeated letter.
fn store_hash(letter: char) -> String {
    String::from(letter).repeat(32)
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

/// Push one path (NAR then narinfo) as `who`.
async fn push(
    service: &Service<MemoryBlobStore, MemoryIndex, &'static TestEnv>,
    who: &Principal,
    cache: &str,
    letter: char,
    nar: &'static [u8],
) {
    service
        .put_nar(who, cache, &nar_name(nar), None, body(nar))
        .await
        .unwrap();
    service
        .put_narinfo(who, cache, &store_hash(letter), &narinfo(letter, nar))
        .await
        .unwrap();
}

async fn read_nar(
    service: &Service<MemoryBlobStore, MemoryIndex, &'static TestEnv>,
    who: &Principal,
    cache: &str,
    nar: &[u8],
) -> Option<Vec<u8>> {
    let (size, mut reader) = service
        .open_nar(who, cache, &nar_name(nar))
        .await
        .unwrap()?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).await.unwrap();
    assert_eq!(size, u64::try_from(out.len()).unwrap());
    Some(out)
}

#[tokio::test]
async fn pushed_path_is_served_back_signed() {
    let service = service();
    push(&service, &writer("pub"), "pub", 'a', b"nar-a").await;

    let anyone = Principal::Anonymous;
    let stored = service
        .narinfo(&anyone, "pub", &store_hash('a'))
        .await
        .unwrap()
        .unwrap();
    assert!(stored.contains("Sig: pub-key:"), "{stored}");
    assert_eq!(
        read_nar(&service, &anyone, "pub", b"nar-a").await.unwrap(),
        b"nar-a"
    );
    assert_eq!(
        service
            .nar_size(&anyone, "pub", &nar_name(b"nar-a"))
            .await
            .unwrap(),
        Some(5)
    );
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('z'))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .cache_info(&anyone, "pub")
            .unwrap()
            .render()
            .starts_with("StoreDir: /nix/store\n")
    );
}

#[tokio::test]
async fn access_follows_visibility_and_grants() {
    let service = service();
    let anyone = Principal::Anonymous;
    let hash = store_hash('a');
    push(&service, &Principal::System, "priv", 'a', b"secret").await;

    // Public: anonymous may read, never write.
    assert!(matches!(
        service
            .put_nar(&anyone, "pub", &nar_name(b"x"), None, body(b"x"))
            .await,
        Err(ServiceError::Unauthenticated)
    ));
    assert!(matches!(
        service
            .put_nar(
                &token("pub", Permission::Read),
                "pub",
                &nar_name(b"x"),
                None,
                body(b"x")
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    // Private: nothing without a grant on that cache.
    assert!(matches!(
        service.narinfo(&anyone, "priv", &hash).await,
        Err(ServiceError::Unauthenticated)
    ));
    assert!(matches!(
        service
            .open_nar(&writer("pub"), "priv", &nar_name(b"secret"))
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        service.cache_info(&anyone, "priv"),
        Err(ServiceError::Unauthenticated)
    ));
    let reader = token("priv", Permission::Read);
    assert!(
        service
            .narinfo(&reader, "priv", &hash)
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        service
            .put_narinfo(&reader, "priv", &hash, &narinfo('a', b"secret"))
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        service.narinfo(&anyone, "nope", &hash).await,
        Err(ServiceError::UnknownCache)
    ));
}

#[tokio::test]
async fn caches_do_not_see_each_others_objects() {
    let service = service();
    push(&service, &Principal::System, "priv", 'a', b"shared-bytes").await;

    let anyone = Principal::Anonymous;
    // Why: the blob exists globally, but `pub` never linked it.
    assert!(
        read_nar(&service, &anyone, "pub", b"shared-bytes")
            .await
            .is_none()
    );
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('a'))
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        service
            .put_narinfo(
                &writer("pub"),
                "pub",
                &store_hash('a'),
                &narinfo('a', b"shared-bytes")
            )
            .await,
        Err(ServiceError::NarNotUploaded)
    ));

    // Each cache signs with its own key.
    push(&service, &writer("pub"), "pub", 'a', b"shared-bytes").await;
    let public = service
        .narinfo(&anyone, "pub", &store_hash('a'))
        .await
        .unwrap()
        .unwrap();
    let private = service
        .narinfo(&Principal::System, "priv", &store_hash('a'))
        .await
        .unwrap()
        .unwrap();
    assert!(public.contains("Sig: pub-key:") && !public.contains("priv-key"));
    assert!(private.contains("Sig: priv-key:") && !private.contains("pub-key"));
    // The bytes are stored once.
    assert_eq!(service.blobs.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn uploads_are_validated() {
    let service = service();
    let who = writer("pub");
    // Bytes must hash to the file name.
    assert!(matches!(
        service
            .put_nar(&who, "pub", &nar_name(b"claimed"), None, body(b"actual"))
            .await,
        Err(ServiceError::HashMismatch)
    ));
    assert!(service.blobs.list().await.unwrap().is_empty());
    assert!(matches!(
        service
            .put_nar(&who, "pub", "../../etc/passwd", None, body(b"x"))
            .await,
        Err(ServiceError::Nix(_))
    ));
    let broken = stream::iter([Err(io::Error::other("gone"))]);
    assert!(matches!(
        service
            .put_nar(&who, "pub", &nar_name(b"x"), None, broken)
            .await,
        Err(ServiceError::Interrupted)
    ));

    service
        .put_nar(&who, "pub", &nar_name(b"good"), None, body(b"good"))
        .await
        .unwrap();
    // The URL hash must be the StorePath's hash.
    assert!(matches!(
        service
            .put_narinfo(&who, "pub", &store_hash('b'), &narinfo('a', b"good"))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    // Declared hashes must match the stored file.
    let lying = narinfo('a', b"good").replace("FileSize: 4", "FileSize: 5");
    assert!(matches!(
        service
            .put_narinfo(&who, "pub", &store_hash('a'), &lying)
            .await,
        Err(ServiceError::Nix(_))
    ));
    assert!(matches!(
        service
            .put_narinfo(&who, "pub", "short", &narinfo('a', b"good"))
            .await,
        Err(ServiceError::Nix(_))
    ));
}

#[tokio::test]
async fn quota_and_size_limits_reject_uploads() {
    let (service, _) = service_with(10, None);
    let who = writer("pub");
    service
        .put_nar(&who, "pub", &nar_name(b"123456"), None, body(b"123456"))
        .await
        .unwrap();
    // Refused up front from the declared length…
    assert!(matches!(
        service
            .put_nar(&who, "pub", &nar_name(b"12345"), Some(5), body(b"12345"))
            .await,
        Err(ServiceError::QuotaExceeded)
    ));
    // …and while streaming when no length was declared.
    assert!(matches!(
        service
            .put_nar(&who, "pub", &nar_name(b"12345"), None, body(b"12345"))
            .await,
        Err(ServiceError::QuotaExceeded)
    ));
    service
        .put_nar(&who, "pub", &nar_name(b"1234"), None, body(b"1234"))
        .await
        .unwrap();
    assert!(matches!(
        service
            .put_nar(&who, "pub", &nar_name(b"1"), None, body(b"1"))
            .await,
        Err(ServiceError::QuotaExceeded)
    ));
    assert_eq!(service.blobs.list().await.unwrap().len(), 2);

    let capped = service_with(0, None).0.with_max_nar_bytes(3);
    assert!(matches!(
        capped
            .put_nar(&who, "pub", &nar_name(b"1234"), None, body(b"1234"))
            .await,
        Err(ServiceError::TooLarge)
    ));
    assert!(matches!(
        capped
            .put_nar(&who, "pub", &nar_name(b"1234"), Some(4), body(b"1234"))
            .await,
        Err(ServiceError::TooLarge)
    ));
}

#[tokio::test]
async fn tokens_are_minted_used_and_revoked() {
    let service = service();
    let root = service.authenticate(Some(ROOT_SECRET)).await.unwrap();
    assert!(root.authorize_admin().is_ok());
    assert_eq!(
        service.authenticate(None).await.unwrap(),
        Principal::Anonymous
    );
    assert!(matches!(
        service.authenticate(Some("wrong")).await,
        Err(ServiceError::Unauthenticated)
    ));

    let (record, secret) = service
        .create_token(&root, NewToken::new("ci", false, vec![("priv", "write")]))
        .await
        .unwrap();
    assert_eq!(record.name(), "ci");
    let ci = service.authenticate(Some(secret.as_str())).await.unwrap();
    push(&service, &ci, "priv", 'a', b"from-ci").await;
    assert!(ci.authorize_admin().is_err());

    // Only admins manage tokens; requests are validated.
    assert!(matches!(
        service.list_tokens(&ci).await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        service
            .create_token(&Principal::Anonymous, NewToken::new("x", true, vec![]))
            .await,
        Err(ServiceError::Unauthenticated)
    ));
    assert!(matches!(
        service
            .create_token(&root, NewToken::new("x", false, vec![("nope", "read")]))
            .await,
        Err(ServiceError::UnknownCache)
    ));
    assert!(matches!(
        service
            .create_token(&root, NewToken::new("x", false, vec![("pub", "own")]))
            .await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(matches!(
        service
            .create_token(&root, NewToken::new("", false, vec![]))
            .await,
        Err(ServiceError::Invalid(_))
    ));

    // A second admin can be minted; revocation takes effect immediately.
    let (_, admin_secret) = service
        .create_token(&root, NewToken::new("ops", true, vec![]))
        .await
        .unwrap();
    let ops = service
        .authenticate(Some(admin_secret.as_str()))
        .await
        .unwrap();
    assert_eq!(service.list_tokens(&ops).await.unwrap().len(), 2);
    assert!(service.revoke_token(&ops, record.id()).await.unwrap());
    assert!(!service.revoke_token(&ops, record.id()).await.unwrap());
    assert!(matches!(
        service.authenticate(Some(secret.as_str())).await,
        Err(ServiceError::Unauthenticated)
    ));
    assert!(matches!(
        service.usage(&ci, "priv").await,
        Err(ServiceError::Forbidden)
    ));
    assert_eq!(service.usage(&ops, "priv").await.unwrap().narinfos(), 1);
}

#[tokio::test]
async fn gc_expires_unread_paths_and_frees_their_blobs() {
    let (service, env) = service_with(0, Some(30 * DAY));
    let who = writer("pub");
    push(&service, &who, "pub", 'a', b"old").await;
    env.now.fetch_add(20 * DAY, Ordering::Relaxed);
    push(&service, &who, "pub", 'b', b"new").await;
    // Private cache has no retention: its copy of the same bytes must survive.
    push(&service, &Principal::System, "priv", 'a', b"old").await;

    env.now.fetch_add(15 * DAY, Ordering::Relaxed);
    let report = service.collect_garbage().await.unwrap();
    assert_eq!((report.expired(), report.blobs_deleted()), (1, 0));
    let anyone = Principal::Anonymous;
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('a'))
            .await
            .unwrap()
            .is_none()
    );
    assert!(read_nar(&service, &anyone, "pub", b"old").await.is_none());
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('b'))
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        read_nar(&service, &Principal::System, "priv", b"old")
            .await
            .unwrap(),
        b"old"
    );

    // Reading 'b' refreshed it, so only time since that read counts.
    env.now.fetch_add(20 * DAY, Ordering::Relaxed);
    assert_eq!(service.collect_garbage().await.unwrap().expired(), 0);
    env.now.fetch_add(11 * DAY, Ordering::Relaxed);
    let report = service.collect_garbage().await.unwrap();
    assert_eq!(
        (
            report.expired(),
            report.blobs_deleted(),
            report.bytes_freed()
        ),
        (1, 1, 3)
    );
    assert_eq!(service.blobs.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn gc_evicts_least_recently_read_paths_when_nearly_full() {
    let (service, env) = service_with(100, None);
    let who = writer("pub");
    let nars: [(char, &'static [u8]); 4] = [
        ('a', &[b'a'; 24]),
        ('b', &[b'b'; 24]),
        ('c', &[b'c'; 24]),
        ('d', &[b'd'; 24]),
    ];
    for (letter, nar) in nars {
        push(&service, &who, "pub", letter, nar).await;
        env.now.fetch_add(2 * 3600, Ordering::Relaxed);
    }
    // 96 of 100 bytes used: under quota, nothing rejected, but above 90%.
    let anyone = Principal::Anonymous;
    // Reading 'a' makes it the most recently used.
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('a'))
            .await
            .unwrap()
            .is_some()
    );

    let report = service.collect_garbage().await.unwrap();
    // Down to <= 75 bytes: one 24-byte path goes, and it is 'b', the stalest.
    assert_eq!(
        (
            report.evicted(),
            report.blobs_deleted(),
            report.bytes_freed()
        ),
        (1, 1, 24)
    );
    assert!(
        service
            .narinfo(&anyone, "pub", &store_hash('b'))
            .await
            .unwrap()
            .is_none()
    );
    for kept in ['a', 'c', 'd'] {
        assert!(
            service
                .narinfo(&anyone, "pub", &store_hash(kept))
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        service.collect_garbage().await.unwrap(),
        super::GcReport::default()
    );
}

#[tokio::test]
async fn gc_removes_nars_whose_narinfo_never_arrived() {
    let (service, env) = service_with(0, None);
    let who = writer("pub");
    service
        .put_nar(&who, "pub", &nar_name(b"orphan"), None, body(b"orphan"))
        .await
        .unwrap();
    // Within the grace period the upload is left alone.
    env.now.fetch_add(DAY / 2, Ordering::Relaxed);
    assert_eq!(
        service.collect_garbage().await.unwrap(),
        super::GcReport::default()
    );
    assert!(
        read_nar(&service, &Principal::Anonymous, "pub", b"orphan")
            .await
            .is_some()
    );

    env.now.fetch_add(DAY, Ordering::Relaxed);
    let report = service.collect_garbage().await.unwrap();
    assert_eq!((report.unlinked_nars(), report.blobs_deleted()), (1, 1));
    assert!(service.blobs.list().await.unwrap().is_empty());
}
