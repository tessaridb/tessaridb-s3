//! Healing on the RS(4, 2) cluster of the `erasure` module: an object listed in `heals` has every missing or damaged
//! shard rewritten from the others, byte for byte as it was written, and leaves the list; one with fewer than four
//! readable shards stays listed; a listing whose data is already gone is dropped.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::erasure::count;
use crate::erasure_reads::{read_back, shard_file, striped, written};
use crate::{IGNORED, call};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_lost_shard_is_rewritten_as_it_was() {
    let _why = IGNORED;
    let member = written().await;
    let path = shard_file(&member, 1);
    let original = std::fs::read(&path).expect("shard read");
    std::fs::remove_file(&path).expect("shard removed");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert_eq!(std::fs::read(&path).expect("shard back"), original);
    assert_eq!(count(&member.planter, "heals").await, 0, "unlisted");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_damaged_shard_is_rewritten_as_it_was() {
    let _why = IGNORED;
    let member = written().await;
    let path = shard_file(&member, 0);
    let original = std::fs::read(&path).expect("shard read");
    let mut damaged = original.clone();
    let byte = damaged.get_mut(132).expect("long enough");
    *byte ^= 0xff;
    std::fs::write(&path, damaged).expect("shard damaged");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert_eq!(std::fs::read(&path).expect("shard back"), original);
    assert_eq!(count(&member.planter, "heals").await, 0, "unlisted");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_below_the_read_quorum_stays_listed_at_risk() {
    let _why = IGNORED;
    let member = written().await;
    for index in [0, 3, 5] {
        std::fs::remove_file(shard_file(&member, index)).expect("shard removed");
    }
    let read = call(&member.state, "GET", "/shards/big", vec![], b"").await;
    assert_eq!(read.status, 503, "{}", read.body);
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 0, 1));
    assert_eq!(count(&member.planter, "heals").await, 1, "still listed");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listing_whose_data_is_gone_is_dropped() {
    let _why = IGNORED;
    let member = written().await;
    member
        .planter
        .run(
            "UPSERT heals:$data SET data = $data, queued = time::now();",
            vec![("data".to_owned(), Value::Uuid([7; 16]))],
        )
        .await
        .expect("planted");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert_eq!(count(&member.planter, "heals").await, 0, "dropped");
}
