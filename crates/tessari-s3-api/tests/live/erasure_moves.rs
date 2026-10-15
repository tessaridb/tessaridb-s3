//! Moving data onto a new layout: once a layout is applied, the mover copies each object's shards that the new
//! placement puts elsewhere, verifies them, switches the object to the new layout, and only later — a pass after, past
//! the reclaim grace — removes the shards the old placement left behind. A crash at any point resumes from the store.

use std::num::NonZeroU64;

use tessari_s3_core::erasure::Code;
use tessari_s3_infrastructure::tessaridb::{Answer, Number, Value};
use tessari_s3_storage::Storage;
use tessari_s3_storage::cluster::{Apply, Stage};
use tessari_s3_types::NodeId;

use crate::IGNORED;
use crate::erasure::{Member, count, peer, shard_files};
use crate::erasure_heals::data_id;
use crate::erasure_reads::{read_back, striped, written};

fn code() -> Code {
    Code::new(4, 2).expect("a code")
}

/// Six more peers, `n7`..`n12`, registered and served.
pub(crate) async fn grown(member: &mut Member) {
    for n in 7..=12 {
        let added = peer(&member.config, &member.authority, &format!("n{n}")).await;
        crate::joined(
            &member.storage,
            &NodeId::new(&format!("n{n}")).expect("an id"),
            &added.address,
            None,
        )
        .await;
        member.peers.push(added);
    }
}

/// Applies a layout of the erasure sets `sets`.
pub(crate) async fn applied(member: &Member, sets: &[&[&str]]) {
    let set = sets
        .iter()
        .map(|nodes| {
            nodes
                .iter()
                .map(|id| NodeId::new(id).expect("an id"))
                .collect()
        })
        .collect();
    let Stage::Staged(staged) = member
        .storage
        .cluster()
        .stage(code(), set)
        .await
        .expect("staged")
    else {
        panic!("not staged");
    };
    assert!(matches!(
        member
            .storage
            .cluster()
            .apply(code(), &staged.id)
            .await
            .expect("applied"),
        Apply::Applied(_)
    ));
}

/// Shard files per node: this node, then its peers in order.
pub(crate) fn spread(member: &Member) -> Vec<usize> {
    std::iter::once(&member.dir)
        .chain(member.peers.iter().map(|peer| &peer.dir))
        .map(|dir| shard_files(dir))
        .collect()
}

pub(crate) async fn layout_of(member: &Member) -> i64 {
    let id = data_id(member).await;
    let answers = member
        .planter
        .run(
            "SELECT layout FROM ONLY shard_sets:$data;",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("read");
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => match records.into_iter().next() {
            Some((_, Value::Object(fields))) => match fields.get("layout") {
                Some(Value::Number(Number::Integer(layout))) => *layout,
                other => panic!("no layout: {other:?}"),
            },
            other => panic!("no record: {other:?}"),
        },
        other => panic!("not records: {other:?}"),
    }
}

/// A budget no test object reaches: the copy is not paced.
pub(crate) const UNPACED: NonZeroU64 = NonZeroU64::MAX;

pub(crate) const NEW_SET: [&str; 6] = ["n7", "n8", "n9", "n10", "n11", "n12"];
pub(crate) const BEFORE: [usize; 12] = [1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0];
const AFTER: [usize; 12] = [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_shard_moves_onto_a_new_set_and_the_old_nodes_are_emptied_a_pass_later() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    assert_eq!(spread(&member), BEFORE);
    applied(&member, &[&NEW_SET]).await;

    let first = member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!((first.examined, first.moved, first.retired), (1, 1, 0));
    assert_eq!(layout_of(&member).await, 2, "switched");
    assert_eq!(
        spread(&member),
        [1; 12],
        "the old shards stay until the retire is due, so a read already under way finds them"
    );
    assert_eq!(read_back(&member).await, striped());

    let second = member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!((second.examined, second.moved, second.retired), (0, 0, 1));
    assert_eq!(spread(&member), AFTER);
    assert_eq!(count(&member.planter, "layout_retire").await, 0);
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_move_cut_short_on_either_side_of_the_switch_finishes_from_the_store() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    // As if the node stopped after copying and before switching: the object is back on version 1, no retire.
    let id = data_id(&member).await;
    member
        .planter
        .run(
            "UPDATE shard_sets:$data SET layout = 1; DELETE layout_retire:$data;",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("rolled back");
    // A node that starts afresh: nothing in memory, only what the store says.
    let restarted = Storage::clustered(
        tessari_s3_infrastructure::tessaridb::MetaPool::new(member.config.meta.clone())
            .expect("pool"),
        &member.config.region,
        member.config.data_dir.clone(),
        member.config.cluster.as_ref().expect("clustered"),
    )
    .expect("clustered storage")
    .with_reclaim_grace(member.config.reclaim_grace_secs);
    let again = restarted
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!(
        (again.examined, again.moved),
        (1, 1),
        "copied again: the same bytes"
    );
    assert_eq!(layout_of(&member).await, 2);
    let last = restarted
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!(
        last.retired, 1,
        "the retire the switch recorded is finished by whoever runs next"
    );
    assert_eq!(spread(&member), AFTER);
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_another_node_holds_is_left_on_its_layout() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    let id = data_id(&member).await;
    crate::erasure_heals::claim_elsewhere(&member.planter, id, true).await;
    let pass = member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!((pass.examined, pass.moved, pass.held), (1, 0, 1));
    assert_eq!(layout_of(&member).await, 1);
    assert_eq!(spread(&member), BEFORE);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn growing_to_two_sets_leaves_each_shard_only_where_the_new_placement_wants_it() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&["n1", "n2", "n3", "n4", "n5", "n6"], &NEW_SET]).await;
    member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    member
        .storage
        .objects()
        .move_layouts(0, 100, UNPACED)
        .await
        .expect("a pass");
    assert_eq!(layout_of(&member).await, 2);
    let after = spread(&member);
    assert!(
        after == BEFORE || after == AFTER,
        "the object is wholly on one of the two sets: {after:?}"
    );
    assert_eq!(count(&member.planter, "layout_retire").await, 0);
    assert_eq!(read_back(&member).await, striped());
}
