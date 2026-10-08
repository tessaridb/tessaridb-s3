use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use tessari_s3_core::authz::{Role, SpaceName, UserPrincipal};
use tessari_s3_types::SecretKey;

use super::{Fetched, Principals, WINDOW_SECS};

fn ann() -> UserPrincipal {
    UserPrincipal {
        name: "ann".to_owned(),
        space: SpaceName::new("alpha").expect("a space"),
        role: Role::Member,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
        grants: BTreeMap::new(),
    }
}

/// Resolves `id` at `now`, counting store lookups and answering `answer` when one is made.
async fn resolve(
    cache: &Principals,
    id: &str,
    now: i64,
    lookups: &AtomicUsize,
    answer: fn() -> Fetched,
) -> bool {
    cache
        .resolve(id, now, || async {
            lookups.fetch_add(1, Ordering::Relaxed);
            Ok::<_, ()>(answer())
        })
        .await
        .expect("no store failure")
        .is_some()
}

fn live() -> Fetched {
    Fetched::Live {
        principal: ann(),
        secret: SecretKey::new("s".repeat(40)),
    }
}

#[tokio::test]
async fn a_resolution_is_believed_for_the_window_and_looked_up_again_after_it() {
    let cache = Principals::new();
    let lookups = AtomicUsize::new(0);
    assert!(resolve(&cache, "TSA", 100, &lookups, live).await);
    assert!(
        resolve(&cache, "TSA", 100 + WINDOW_SECS - 1, &lookups, || {
            Fetched::Unknown
        })
        .await
    );
    assert_eq!(
        lookups.load(Ordering::Relaxed),
        1,
        "inside the window the store is not asked"
    );
    assert!(
        !resolve(&cache, "TSA", 100 + WINDOW_SECS, &lookups, || {
            Fetched::Unknown
        })
        .await
    );
    assert_eq!(lookups.load(Ordering::Relaxed), 2, "at the window it is");
}

#[tokio::test]
async fn a_key_that_does_not_resolve_is_never_cached() {
    let cache = Principals::new();
    let unknown = AtomicUsize::new(0);
    for _ in 0..3 {
        assert!(!resolve(&cache, "TSU", 100, &unknown, || Fetched::Unknown).await);
    }
    assert_eq!(
        unknown.load(Ordering::Relaxed),
        3,
        "every unknown id is a lookup, never an entry"
    );
}

#[tokio::test]
async fn a_clock_that_moved_back_is_not_trusted_to_keep_an_entry() {
    let cache = Principals::new();
    let lookups = AtomicUsize::new(0);
    assert!(resolve(&cache, "TSA", 100, &lookups, live).await);
    assert!(!resolve(&cache, "TSA", 99, &lookups, || Fetched::Unknown).await);
    assert_eq!(lookups.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_failed_lookup_serves_nothing_stale() {
    let cache = Principals::new();
    let lookups = AtomicUsize::new(0);
    assert!(resolve(&cache, "TSA", 100, &lookups, live).await);
    let failed = cache
        .resolve("TSA", 100 + WINDOW_SECS, || async {
            Err::<Fetched, _>("store down")
        })
        .await;
    assert!(matches!(failed, Err("store down")));
}
