//! One behavioral contract every `Index` adapter must satisfy.
//! Why: the memory and `SQLite` indexes must be interchangeable.

use crate::adapters::memory::MemoryIndex;
use crate::application::ports::{Index, NarRecord, Usage};
use crate::domain::access::{Grant, Permission, TokenRecord};
use crate::domain::cache::CacheName;
use crate::domain::digest::Digest;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn nar(content: &[u8]) -> NarRecord {
    NarRecord::new(Digest::of(content), u64::try_from(content.len()).unwrap())
}

fn token(id: &str, secret: &[u8], created_at: i64) -> TokenRecord {
    TokenRecord::new(
        String::from(id),
        format!("name-{id}"),
        Digest::of(secret),
        false,
        vec![Grant::new(
            CacheName::parse("one").unwrap(),
            Permission::Write,
        )],
        created_at,
        None,
    )
}

/// Write the rows `check_seeded` expects.
pub async fn seed<I: Index>(index: &I) {
    index.insert_token(&token("t1", b"s1", 10)).await.unwrap();
    index
        .link_nar("one", "x.nar", nar(b"xx"), 100)
        .await
        .unwrap();
    assert!(
        index
            .put_narinfo("one", A, "x.nar", "body-a", 100)
            .await
            .unwrap()
    );
}

/// Read back what `seed` wrote.
pub async fn check_seeded<I: Index>(index: &I) {
    index.ping().await.unwrap();
    let found = index
        .token_by_hash(&Digest::of(b"s1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found, token("t1", b"s1", 10));
    assert_eq!(index.nar("one", "x.nar").await.unwrap(), Some(nar(b"xx")));
    assert_eq!(
        index.narinfo("one", A).await.unwrap().as_deref(),
        Some("body-a")
    );
    assert_eq!(index.usage("one").await.unwrap(), Usage::new(2, 1, 1));
}

/// The full contract, against an empty index.
pub async fn check<I: Index>(index: &I) {
    seed(index).await;
    check_seeded(index).await;

    // Tokens: lookup, listing order, single revocation.
    index.insert_token(&token("t2", b"s2", 20)).await.unwrap();
    assert!(
        index
            .token_by_hash(&Digest::of(b"nope"))
            .await
            .unwrap()
            .is_none()
    );
    let ids: Vec<String> = index
        .tokens()
        .await
        .unwrap()
        .iter()
        .map(|t| String::from(t.id()))
        .collect();
    assert_eq!(ids, ["t1", "t2"]);
    assert!(index.revoke_token("t1", 30).await.unwrap());
    assert!(!index.revoke_token("t1", 31).await.unwrap());
    assert!(!index.revoke_token("missing", 31).await.unwrap());
    let revoked = index
        .token_by_hash(&Digest::of(b"s1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(revoked.revoked_at(), Some(30));

    // Caches are isolated; a narinfo needs its NAR in the same cache.
    assert!(index.nar("two", "x.nar").await.unwrap().is_none());
    assert!(index.narinfo("two", A).await.unwrap().is_none());
    assert!(
        !index
            .put_narinfo("two", A, "x.nar", "body", 100)
            .await
            .unwrap()
    );
    assert!(
        !index
            .put_narinfo("one", B, "missing.nar", "body", 100)
            .await
            .unwrap()
    );
    assert_eq!(index.usage("two").await.unwrap(), Usage::default());

    // Re-putting replaces the body; a second path may share the NAR.
    assert!(
        index
            .put_narinfo("one", A, "x.nar", "body-a2", 100)
            .await
            .unwrap()
    );
    assert_eq!(
        index.narinfo("one", A).await.unwrap().as_deref(),
        Some("body-a2")
    );
    assert!(
        index
            .put_narinfo("one", B, "x.nar", "body-b", 200)
            .await
            .unwrap()
    );
    assert_eq!(index.usage("one").await.unwrap(), Usage::new(2, 1, 2));

    check_lifecycle(index).await;
}

/// Staleness, deletion, and pruning, continuing from `check`'s state:
/// cache `one` holds `x.nar` claimed by `A` (accessed 100) and `B` (200).
async fn check_lifecycle<I: Index>(index: &I) {
    // Staleness: ordered by last access, filtered by cutoff, limited.
    assert_eq!(index.stale_narinfos("one", None, 10).await.unwrap(), [A, B]);
    assert_eq!(index.stale_narinfos("one", None, 1).await.unwrap(), [A]);
    assert_eq!(
        index.stale_narinfos("one", Some(150), 10).await.unwrap(),
        [A]
    );
    assert!(
        index
            .stale_narinfos("one", Some(50), 10)
            .await
            .unwrap()
            .is_empty()
    );
    // Touch only moves entries older than the threshold.
    index.touch_narinfo("one", A, 300, 50).await.unwrap();
    assert_eq!(index.stale_narinfos("one", None, 10).await.unwrap(), [A, B]);
    index.touch_narinfo("one", A, 300, 150).await.unwrap();
    assert_eq!(index.stale_narinfos("one", None, 10).await.unwrap(), [B, A]);

    // Deleting one of two paths keeps the shared NAR; the last frees it.
    assert!(index.blob_referenced(&Digest::of(b"xx")).await.unwrap());
    assert_eq!(index.delete_narinfo("one", B).await.unwrap(), 0);
    assert_eq!(index.delete_narinfo("one", B).await.unwrap(), 0);
    assert_eq!(index.delete_narinfo("one", A).await.unwrap(), 2);
    assert!(index.nar("one", "x.nar").await.unwrap().is_none());
    assert!(!index.blob_referenced(&Digest::of(b"xx")).await.unwrap());

    // Pruning drops only old NARs no narinfo claims.
    index
        .link_nar("one", "old.nar", nar(b"old"), 100)
        .await
        .unwrap();
    index
        .link_nar("one", "new.nar", nar(b"new"), 500)
        .await
        .unwrap();
    index
        .link_nar("one", "used.nar", nar(b"used"), 100)
        .await
        .unwrap();
    assert!(
        index
            .put_narinfo("one", A, "used.nar", "body", 100)
            .await
            .unwrap()
    );
    assert_eq!(index.prune_unlinked_nars(400).await.unwrap(), 1);
    assert!(index.nar("one", "old.nar").await.unwrap().is_none());
    assert!(index.nar("one", "new.nar").await.unwrap().is_some());
    assert!(index.nar("one", "used.nar").await.unwrap().is_some());
    // Re-linking refreshes the age, protecting a re-uploaded NAR.
    index
        .link_nar("one", "new.nar", nar(b"new"), 900)
        .await
        .unwrap();
    assert_eq!(index.prune_unlinked_nars(800).await.unwrap(), 0);
}

#[tokio::test]
async fn memory_index_satisfies_the_contract() {
    check(&MemoryIndex::new()).await;
}
