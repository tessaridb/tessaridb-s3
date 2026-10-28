//! Data no object holds yet — a write refused below its quorum, cut short or never committed — is not moved onto a
//! new layout and not counted as moving: its shard set was recorded before any shard, and the reclaim queue, which
//! names it until a commit takes it, removes it once the grace has run.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};

use crate::IGNORED;
use crate::erasure_heals::data_id;
use crate::erasure_moves::{NEW_SET, UNPACED, applied, grown, layout_of};
use crate::erasure_reads::{read_back, striped, written};

/// A shard set and its reclaim entry with no shard and no object, as a write that stopped after queuing leaves them.
async fn uncommitted(planter: &MetaPool) {
    planter
        .run(
            "BEGIN; UPSERT reclaims:$data SET data = $data, due = time::now() + duration::from_secs(86400); \
             CREATE shard_sets:$data = { layout: 1, size: 3145728 }; COMMIT;",
            vec![("data".to_owned(), Value::Uuid(uuid::Uuid::new_v4().into_bytes()))],
        )
        .await
        .expect("planted");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_uncommitted_write_is_neither_moved_nor_counted_as_moving() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    uncommitted(&member.planter).await;
    applied(&member, &[&NEW_SET]).await;
    let objects = member.storage.objects();

    let before = objects
        .moving(100)
        .await
        .expect("backlog")
        .expect("clustered");
    assert_eq!(
        (before.listed, before.more),
        (1, false),
        "only the object is moving"
    );
    let pass = objects.move_layouts(0, 100, UNPACED).await.expect("a pass");
    assert_eq!((pass.examined, pass.moved), (1, 1));
    let after = objects
        .moving(100)
        .await
        .expect("backlog")
        .expect("clustered");
    assert_eq!((after.listed, after.more), (0, false), "the move is over");
    assert_eq!(layout_of(&member).await, 2);
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn data_queued_for_reclaim_that_an_object_still_holds_is_moved() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    // As superseded data a version still holds is queued, until the reclaimer finds it held.
    member
        .planter
        .run(
            "UPSERT reclaims:$data SET data = $data, due = time::now() + duration::from_secs(86400);",
            vec![("data".to_owned(), Value::Uuid(data_id(&member).await))],
        )
        .await
        .expect("queued");
    applied(&member, &[&NEW_SET]).await;
    let objects = member.storage.objects();

    let before = objects
        .moving(100)
        .await
        .expect("backlog")
        .expect("clustered");
    assert_eq!(
        (before.listed, before.more),
        (1, false),
        "held data is moving"
    );
    let pass = objects.move_layouts(0, 100, UNPACED).await.expect("a pass");
    assert_eq!((pass.examined, pass.moved), (1, 1));
    assert_eq!(layout_of(&member).await, 2);
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn uncommitted_writes_beyond_a_pass_do_not_hold_back_the_data_behind_them() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    for _ in 0..5 {
        uncommitted(&member.planter).await;
    }
    applied(&member, &[&NEW_SET]).await;
    let objects = member.storage.objects();

    let before = objects
        .moving(2)
        .await
        .expect("backlog")
        .expect("clustered");
    assert_eq!((before.listed, before.more), (1, false));
    let pass = objects.move_layouts(0, 2, UNPACED).await.expect("a pass");
    assert_eq!((pass.examined, pass.moved), (1, 1));
    assert_eq!(layout_of(&member).await, 2);
}
