use tessari_s3_constants::LAYOUT_PROPOSE_ATTEMPTS;
use tessari_s3_core::erasure::Code;
use tessari_s3_infrastructure::tessaridb::RefusalClass;

use super::{current_of, version_of};
use crate::cluster::entity::MemberEntity;
use crate::cluster::fake::{Fake, OnPropose, cache, code, entity, names, plain};

#[tokio::test]
async fn a_proposal_that_lost_the_race_takes_the_winners_layout() {
    // The winner saw a node this one has not seen yet, so the two proposals differ.
    let store = Fake::new(
        vec![("n2", None), ("n3", None), ("n4", None)],
        OnPropose::Loses,
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("asked")
        .expect("a layout");
    assert_eq!(
        names(&layout),
        plain(&[&["n1", "n2", "n3"]]),
        "the committed layout, not this node's proposal"
    );
}

#[tokio::test]
async fn a_proposal_refused_retry_is_proposed_again_and_takes_the_winners_layout() {
    let store = Fake::new(
        vec![("n1", None), ("n2", None), ("n3", None)],
        OnPropose::Contended(1),
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("a contended proposal is retried, not reported")
        .expect("a layout");
    assert_eq!(layout.version(), 1);
    assert_eq!(*store.proposals.lock().expect("unpoisoned"), 2);
}

#[tokio::test]
async fn proposals_refused_retry_every_time_give_up_after_the_bound() {
    let store = Fake::new(
        vec![("n1", None), ("n2", None), ("n3", None)],
        OnPropose::Contended(u32::MAX),
    );
    let outcome = current_of(&store, &cache(), code()).await;
    assert!(
        matches!(&outcome, Err(crate::Error::Meta(error)) if error.is_class(RefusalClass::Retry)),
        "{outcome:?}"
    );
    assert_eq!(
        *store.proposals.lock().expect("unpoisoned"),
        LAYOUT_PROPOSE_ATTEMPTS
    );
}

#[tokio::test]
async fn the_first_layout_follows_the_registered_zones() {
    // 2+1 survives one shard per zone: n2 shares n1's zone, so n3 takes its place.
    let store = Fake::new(
        vec![
            ("n1", Some("a")),
            ("n2", Some("a")),
            ("n3", Some("b")),
            ("n4", Some("c")),
        ],
        OnPropose::Commits,
    );
    let layout = current_of(&store, &cache(), code())
        .await
        .expect("asked")
        .expect("a layout");
    let zoned = |node: &str, zone: &str| (node.to_owned(), Some(zone.to_owned()));
    assert_eq!(
        names(&layout),
        vec![vec![zoned("n1", "a"), zoned("n3", "b"), zoned("n4", "c")]]
    );
    let stored = store.layouts.lock().expect("unpoisoned")[&1].clone();
    assert_eq!(
        stored.sets[0][1],
        MemberEntity {
            node: "n3".to_owned(),
            zone: Some("b".to_owned())
        },
        "the zones are stored with the layout"
    );
}

#[tokio::test]
async fn the_current_version_names_the_layout_and_each_version_is_read_once() {
    let store = Fake::new(vec![], OnPropose::Commits);
    store.commit(&entity(1, &[&["n1", "n2", "n3"]]));
    store.commit(&entity(2, &[&["n4", "n5", "n6"], &["n7", "n8", "n9"]]));
    let layouts = cache();
    for _ in 0..3 {
        let layout = current_of(&store, &layouts, code())
            .await
            .expect("asked")
            .expect("a layout");
        assert_eq!(
            (layout.version(), names(&layout)),
            (2, plain(&[&["n4", "n5", "n6"], &["n7", "n8", "n9"]]))
        );
    }
    let first = version_of(&store, &layouts, code(), 1)
        .await
        .expect("version 1 exists");
    assert_eq!(names(&first), plain(&[&["n1", "n2", "n3"]]));
    assert_eq!(
        *store.layout_reads.lock().expect("unpoisoned"),
        2,
        "one read per version, then the cache"
    );
    assert!(
        matches!(
            version_of(&store, &layouts, Code::new(3, 1).expect("a code"), 1).await,
            Err(crate::Error::LayoutMismatch { .. })
        ),
        "a cached layout is held to the asking node's code as a stored one is"
    );
    assert!(matches!(
        version_of(&store, &layouts, code(), 3).await,
        Err(crate::Error::Malformed {
            record: "layout",
            ..
        })
    ));
}
