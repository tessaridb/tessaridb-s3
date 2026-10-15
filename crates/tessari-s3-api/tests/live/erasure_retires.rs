//! Retiring what a move left behind: a shard the new placement keeps on its node is never removed, an object deleted
//! before its retire leaves none of its old shards, and a later layout does not switch an id whose retire is pending.

use crate::erasure::count;
use crate::erasure_moves::{BEFORE, NEW_SET, applied, grown, layout_of, spread};
use crate::erasure_reads::{read_back, striped, written};
use crate::{IGNORED, call};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_shard_the_new_set_keeps_on_its_node_is_neither_copied_nor_retired() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    // Five of the six nodes stay: whatever shard index each keeps, its one file must survive the retire.
    applied(&member, &[&["n1", "n2", "n3", "n4", "n5", "n7"]]).await;
    let first = member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    assert_eq!((first.examined, first.moved), (1, 1));
    let second = member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    assert_eq!(second.retired, 1);
    assert_eq!(spread(&member), [1, 1, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_deleted_before_its_retire_leaves_no_old_shard_behind() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    let deleted = call(&member.state, "DELETE", "/shards/big", vec![], b"").await;
    assert_eq!(deleted.status, 204);
    member
        .storage
        .objects()
        .reclaim(0, 100)
        .await
        .expect("reclaim pass");
    assert_eq!(
        count(&member.planter, "shard_sets").await,
        0,
        "the object is gone"
    );
    let pass = member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    assert_eq!(pass.retired, 1);
    assert_eq!(spread(&member), [0; 12]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_second_layout_waits_for_the_first_moves_retire() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    applied(&member, &[&["n1", "n2", "n3", "n4", "n5", "n6"]]).await;
    let early = member
        .storage
        .objects()
        .move_layouts(3600, 100)
        .await
        .expect("a pass");
    assert_eq!(
        (early.examined, early.moved, early.retired),
        (1, 0, 0),
        "the retire is not due"
    );
    assert_eq!(
        layout_of(&member).await,
        2,
        "not switched over a pending retire"
    );
    let due = member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    assert_eq!((due.retired, due.moved), (1, 1));
    assert_eq!(layout_of(&member).await, 3);
    member
        .storage
        .objects()
        .move_layouts(0, 100)
        .await
        .expect("a pass");
    assert_eq!(spread(&member), BEFORE);
    assert_eq!(read_back(&member).await, striped());
}
