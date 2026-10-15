use tessari_s3_core::erasure::LayoutError;
use tessari_s3_types::NodeId;

use super::{Apply, Stage, apply_of, stage_of, staged_of};
use crate::cluster::fake::{Fake, OnPropose, cache, code, names};
use crate::cluster::layouts::{Layouts, current_of};

/// Six nodes, two per zone, under RS(2, 1); the first layout (version 1) committed.
async fn cluster() -> (Fake, Layouts) {
    let store = Fake::new(
        vec![
            ("n1", Some("a")),
            ("n2", Some("a")),
            ("n3", Some("b")),
            ("n4", Some("b")),
            ("n5", Some("c")),
            ("n6", Some("c")),
        ],
        OnPropose::Commits,
    );
    let layouts = cache();
    let first = current_of(&store, &layouts, code()).await.expect("read");
    assert_eq!(first.map(|layout| layout.version()), Some(1));
    (store, layouts)
}

fn sets(sets: &[&[&str]]) -> Vec<Vec<NodeId>> {
    sets.iter()
        .map(|set| {
            set.iter()
                .map(|node| NodeId::new(node).expect("an id"))
                .collect()
        })
        .collect()
}

fn zoned(sets: &[&[(&str, &str)]]) -> Vec<Vec<(String, Option<String>)>> {
    sets.iter()
        .map(|set| {
            set.iter()
                .map(|(node, zone)| ((*node).to_owned(), Some((*zone).to_owned())))
                .collect()
        })
        .collect()
}

/// Two sets of three, one node of each zone per set.
const TWO_SETS: &[&[&str]] = &[&["n1", "n3", "n5"], &["n2", "n4", "n6"]];

#[tokio::test]
async fn nothing_is_staged_over_no_layout() {
    let store = Fake::new(vec![("n1", None)], OnPropose::Commits);
    let staged = stage_of(&store, &cache(), code(), sets(&[&["n1"]])).await;
    assert_eq!(staged.expect("answered"), Stage::NoLayoutYet);
    assert_eq!(staged_of(&store, code()).await.expect("read"), None);
}

#[tokio::test]
async fn a_node_that_never_registered_is_named() {
    let (store, layouts) = cluster().await;
    let staged = stage_of(&store, &layouts, code(), sets(&[&["n1", "n3", "n9"]])).await;
    assert_eq!(
        staged.expect("answered"),
        Stage::Unregistered(NodeId::new("n9").expect("an id"))
    );
    assert_eq!(staged_of(&store, code()).await.expect("read"), None);
}

#[tokio::test]
async fn a_set_with_two_shards_in_one_zone_is_refused_by_its_rule() {
    let (store, layouts) = cluster().await;
    let staged = stage_of(&store, &layouts, code(), sets(&[&["n1", "n2", "n3"]])).await;
    match staged.expect("answered") {
        Stage::Invalid(LayoutError::ZoneOverloaded { set: 0, .. }) => {}
        other => panic!("not refused for its zone: {other:?}"),
    }
    assert_eq!(staged_of(&store, code()).await.expect("read"), None);
}

#[tokio::test]
async fn a_staged_layout_is_the_next_version_with_the_registered_zones() {
    let (store, layouts) = cluster().await;
    let Stage::Staged(staged) = stage_of(&store, &layouts, code(), sets(TWO_SETS))
        .await
        .expect("answered")
    else {
        panic!("not staged");
    };
    assert_eq!((staged.base, staged.layout.version()), (1, 2));
    assert_eq!(
        names(&staged.layout),
        zoned(&[
            &[("n1", "a"), ("n3", "b"), ("n5", "c")],
            &[("n2", "a"), ("n4", "b"), ("n6", "c")]
        ])
    );
    assert_eq!(staged_of(&store, code()).await.expect("read"), Some(staged));
}

#[tokio::test]
async fn applying_makes_it_current_and_clears_the_stage() {
    let (store, layouts) = cluster().await;
    let Stage::Staged(staged) = stage_of(&store, &layouts, code(), sets(TWO_SETS))
        .await
        .expect("answered")
    else {
        panic!("not staged");
    };
    *store.apply_refusals.lock().expect("unpoisoned") = 2;
    let applied = apply_of(&store, &layouts, code(), &staged.id).await;
    assert_eq!(
        applied.expect("answered"),
        Apply::Applied(staged.layout.clone())
    );
    let current = current_of(&store, &layouts, code()).await.expect("read");
    assert_eq!(current, Some(staged.layout));
    assert_eq!(staged_of(&store, code()).await.expect("read"), None);
}

#[tokio::test]
async fn an_apply_naming_another_staging_changes_nothing() {
    let (store, layouts) = cluster().await;
    let Stage::Staged(_) = stage_of(&store, &layouts, code(), sets(TWO_SETS))
        .await
        .expect("answered")
    else {
        panic!("not staged");
    };
    let applied = apply_of(&store, &layouts, code(), "some-other-staging").await;
    assert_eq!(applied.expect("answered"), Apply::StagedChanged);
    let current = current_of(&store, &layouts, code()).await.expect("read");
    assert_eq!(current.map(|layout| layout.version()), Some(1));
}

#[tokio::test]
async fn an_apply_over_a_moved_version_changes_nothing() {
    let (store, layouts) = cluster().await;
    let Stage::Staged(staged) = stage_of(&store, &layouts, code(), sets(TWO_SETS))
        .await
        .expect("answered")
    else {
        panic!("not staged");
    };
    // Another apply made version 2 meanwhile.
    store.commit(&crate::cluster::fake::entity(2, &[&["n1", "n3", "n5"]]));
    let applied = apply_of(&store, &layouts, code(), &staged.id).await;
    assert_eq!(applied.expect("answered"), Apply::BaseMoved);
    assert!(staged_of(&store, code()).await.expect("read").is_some());
}

#[tokio::test]
async fn nothing_staged_is_nothing_to_apply() {
    let (store, layouts) = cluster().await;
    let applied = apply_of(&store, &layouts, code(), "any").await;
    assert_eq!(applied.expect("answered"), Apply::NothingStaged);
}

#[tokio::test]
async fn a_staging_replaced_as_the_apply_runs_is_not_applied() {
    let (store, layouts) = cluster().await;
    let Stage::Staged(staged) = stage_of(&store, &layouts, code(), sets(TWO_SETS))
        .await
        .expect("answered")
    else {
        panic!("not staged");
    };
    let mut other = store
        .staged
        .lock()
        .expect("unpoisoned")
        .clone()
        .expect("staged");
    other.id = "another-operators-staging".to_owned();
    *store.staged_before_apply.lock().expect("unpoisoned") = Some(other);
    let applied = apply_of(&store, &layouts, code(), &staged.id).await;
    assert_eq!(applied.expect("answered"), Apply::StagedChanged);
    let current = current_of(&store, &layouts, code()).await.expect("read");
    assert_eq!(current.map(|layout| layout.version()), Some(1));
}
