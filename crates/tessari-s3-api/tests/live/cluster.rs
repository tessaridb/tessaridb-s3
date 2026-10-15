//! The cluster's topology in TessariDB: nodes register themselves, and the first layout is created once, from the
//! lowest registered ids, however many nodes race to propose it.

use tessari_s3_core::erasure::Code;
use tessari_s3_storage::{Error, Storage};
use tessari_s3_types::{NodeId, Zone};

use crate::{IGNORED, fresh};

fn node(id: &str) -> NodeId {
    NodeId::new(id).expect("a node id")
}

fn code(data: u8, parity: u8) -> Code {
    Code::new(data, parity).expect("a code")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_registers_once_and_its_endpoint_follows_it() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let cluster = state.storage().cluster();
    crate::joined(state.storage(), &node("n1"), "a.internal:1", None).await;
    crate::joined(state.storage(), &node("n2"), "b.internal:1", None).await;
    crate::joined(state.storage(), &node("n1"), "c.internal:2", None).await;
    let members = cluster.members().await.expect("members");
    let seen: Vec<(&str, &str)> = members
        .iter()
        .map(|member| (member.node.as_str(), member.endpoint.as_str()))
        .collect();
    assert_eq!(seen, [("n1", "c.internal:2"), ("n2", "b.internal:1")]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_registers_its_zone_and_a_later_registration_replaces_it() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let cluster = state.storage().cluster();
    let zone = |name: &str| Zone::new(name).expect("a zone");
    crate::joined(
        state.storage(),
        &node("n1"),
        "a.internal:1",
        Some(&zone("eu-1a")),
    )
    .await;
    crate::joined(state.storage(), &node("n2"), "b.internal:1", None).await;
    let zones = || async {
        cluster
            .members()
            .await
            .expect("members")
            .into_iter()
            .map(|member| {
                (
                    member.node.as_str().to_owned(),
                    member.zone.map(|zone| zone.as_str().to_owned()),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        zones().await,
        [
            ("n1".to_owned(), Some("eu-1a".to_owned())),
            ("n2".to_owned(), None)
        ]
    );
    crate::joined(state.storage(), &node("n1"), "a.internal:1", None).await;
    crate::joined(
        state.storage(),
        &node("n2"),
        "b.internal:1",
        Some(&zone("eu-1b")),
    )
    .await;
    assert_eq!(
        zones().await,
        [
            ("n1".to_owned(), None),
            ("n2".to_owned(), Some("eu-1b".to_owned()))
        ],
        "the node's own configuration decides, not its first registration"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn racing_nodes_agree_on_one_first_layout() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    let cluster = state.storage().cluster();
    // A second node: its own pool over the same namespace.
    let other = Storage::new(planter, state.region(), crate::scratch_dir()).expect("a second node");
    for id in ["n5", "n3"] {
        crate::joined(state.storage(), &node(id), "x:1", None).await;
    }
    assert_eq!(
        cluster.current(code(2, 1)).await.expect("asked"),
        None,
        "two nodes cannot hold a 2+1 layout"
    );
    for id in ["n9", "n1"] {
        crate::joined(state.storage(), &node(id), "x:1", None).await;
    }
    // Racers on two pools, spawned so they run in parallel; each must come back with the same layout. Which path a
    // racer takes depends on timing, so the lost-race read-back is pinned by the service's unit test instead.
    let racers: Vec<_> = (0..8)
        .map(|turn| {
            let service = if turn % 2 == 0 {
                cluster.clone()
            } else {
                other.cluster().clone()
            };
            tokio::spawn(async move { service.current(code(2, 1)).await })
        })
        .collect();
    let mut answers = Vec::new();
    for racer in racers {
        answers.push(
            racer
                .await
                .expect("joined")
                .expect("asked")
                .expect("a layout"),
        );
    }
    let first = answers[0].clone();
    assert!(answers.iter().all(|answer| *answer == first), "{answers:?}");
    let names: Vec<&str> = first.sets()[0]
        .iter()
        .map(|member| member.node().as_str())
        .collect();
    assert_eq!((first.version(), names), (1, vec!["n1", "n3", "n5"]));
    crate::joined(state.storage(), &node("n0"), "x:1", None).await;
    assert_eq!(
        cluster.current(code(2, 1)).await.expect("asked"),
        Some(first),
        "a later node does not move a layout that exists"
    );
    assert!(
        matches!(
            cluster.current(code(3, 1)).await,
            Err(Error::LayoutMismatch { .. })
        ),
        "a node configured with another code is refused rather than reinterpreting the stored layout"
    );
}
