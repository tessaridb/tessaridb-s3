//! A layout change on a real TessariDB node: staged over the current version, applied as version + 1 by
//! compare-and-set in one transaction, and refused — with nothing written — when the current version moved.

use tessari_s3_core::erasure::{Code, LayoutError};
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::cluster::{Apply, ClusterService, Stage, StagedLayout};
use tessari_s3_types::{NodeId, Zone};

use crate::{IGNORED, fresh};

fn code() -> Code {
    Code::new(2, 1).expect("a code")
}

fn sets(sets: &[&[&str]]) -> Vec<Vec<NodeId>> {
    sets.iter()
        .map(|set| {
            set.iter()
                .map(|id| NodeId::new(id).expect("an id"))
                .collect()
        })
        .collect()
}

/// Six nodes, two per zone, and the first layout (version 1).
async fn cluster() -> (ClusterService, MetaPool) {
    let (state, planter) = fresh().await;
    let cluster = state.storage().cluster().clone();
    for (id, zone) in [
        ("n1", "a"),
        ("n2", "a"),
        ("n3", "b"),
        ("n4", "b"),
        ("n5", "c"),
        ("n6", "c"),
    ] {
        cluster
            .register(
                &NodeId::new(id).expect("an id"),
                "x:1",
                Some(&Zone::new(zone).expect("a zone")),
            )
            .await
            .expect("registered");
    }
    let first = cluster.current(code()).await.expect("asked");
    assert_eq!(first.map(|layout| layout.version()), Some(1));
    (cluster, planter)
}

async fn staged(cluster: &ClusterService) -> StagedLayout {
    match cluster
        .stage(code(), sets(&[&["n1", "n3", "n5"], &["n2", "n4", "n6"]]))
        .await
        .expect("answered")
    {
        Stage::Staged(staged) => staged,
        other => panic!("not staged: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_staged_layout_applies_as_the_next_version_and_the_old_one_stays_readable() {
    let _why = IGNORED;
    let (cluster, _) = cluster().await;
    let staged = staged(&cluster).await;
    assert_eq!((staged.base, staged.layout.version()), (1, 2));
    assert_eq!(
        cluster.staged(code()).await.expect("read"),
        Some(staged.clone())
    );
    let applied = cluster.apply(code(), &staged.id).await.expect("answered");
    assert_eq!(applied, Apply::Applied(staged.layout.clone()));
    assert_eq!(
        cluster.current(code()).await.expect("asked"),
        Some(staged.layout)
    );
    assert_eq!(cluster.staged(code()).await.expect("read"), None);
    assert_eq!(
        cluster.version(code(), 1).await.expect("kept").version(),
        1,
        "objects written under version 1 still resolve it"
    );
    assert_eq!(
        cluster.apply(code(), &staged.id).await.expect("answered"),
        Apply::NothingStaged,
        "applied once"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_apply_over_a_moved_version_writes_nothing() {
    let _why = IGNORED;
    let (cluster, planter) = cluster().await;
    let staged = staged(&cluster).await;
    // Another node applied a change meanwhile: the current version is no longer the base.
    planter
        .run(
            "CREATE layouts:5 = (SELECT * FROM ONLY layouts:1); UPDATE layouts:5 SET version = 5; \
             UPDATE layout_current:'cluster' SET version = 5;",
            Vec::new(),
        )
        .await
        .expect("moved");
    let applied = cluster.apply(code(), &staged.id).await.expect("answered");
    assert_eq!(applied, Apply::BaseMoved);
    let answers = planter
        .run("SELECT version FROM ONLY layouts:2;", Vec::new())
        .await
        .expect("read");
    assert!(
        matches!(
            answers.first(),
            Some(tessari_s3_infrastructure::tessaridb::Answer::Records { records, .. })
                if records.is_empty()
        ),
        "no version 2 was created: {answers:?}"
    );
    assert_eq!(
        cluster
            .staged(code())
            .await
            .expect("read")
            .map(|now| now.id),
        Some(staged.id),
        "the staged layout is still there to stage again"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_layout_that_breaks_a_rule_is_not_staged() {
    let _why = IGNORED;
    let (cluster, _) = cluster().await;
    match cluster
        .stage(code(), sets(&[&["n1", "n2", "n3"]]))
        .await
        .expect("answered")
    {
        Stage::Invalid(LayoutError::ZoneOverloaded { set: 0, .. }) => {}
        other => panic!("not refused for its zone: {other:?}"),
    }
    assert_eq!(
        cluster
            .stage(code(), sets(&[&["n1", "n3", "n7"]]))
            .await
            .expect("answered"),
        Stage::Unregistered(NodeId::new("n7").expect("an id"))
    );
    assert_eq!(cluster.staged(code()).await.expect("read"), None);
}
