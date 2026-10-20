//! Healing on the RS(4, 2) cluster of the `erasure` module: an object the `heal_queue` hands out has every missing or
//! damaged shard rewritten from the others, byte for byte as it was written, and leaves the list; one with fewer than four
//! readable shards stays listed; a listing whose data is already gone is dropped. One node at a time works on a data
//! id: a claim another node holds keeps this node's healer and reclaimer off it until it is released, and shard bytes
//! whose shard set is gone are swept.

use tessari_s3_constants::{HEAL_AT_RISK_ATTEMPTS, HEAL_RETRY_SECS, RECLAIM_RETRY_SECS};
use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};
use tessari_s3_storage::objects::Content;
use tessari_s3_types::{BucketName, ObjectKey};

use crate::erasure::{Member, count, shard_files};
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
    assert_eq!(count(&member.planter, "heal_queue").await, 0, "unlisted");
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
    assert_eq!(count(&member.planter, "heal_queue").await, 0, "unlisted");
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
    assert_eq!(
        count(&member.planter, "heal_queue").await,
        1,
        "still listed"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listing_whose_data_is_gone_is_dropped() {
    let _why = IGNORED;
    let member = written().await;
    member
        .planter
        .run(
            "UPSERT heal_queue:$data SET data = $data, due = time::now();",
            vec![("data".to_owned(), Value::Uuid([7; 16]))],
        )
        .await
        .expect("planted");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert_eq!(count(&member.planter, "heal_queue").await, 0, "dropped");
}

/// The data id `/shards/big` is stored under.
pub(crate) async fn data_id(member: &Member) -> [u8; 16] {
    let object = member
        .storage
        .objects()
        .get(
            &BucketName::new("shards").expect("bucket name"),
            &ObjectKey::new("big").expect("key"),
        )
        .await
        .expect("read")
        .expect("bucket")
        .expect("object");
    let Content::Data(id) = object.content else {
        panic!("a large object has a data id");
    };
    id
}

/// Another node (`n9`) takes, or hands back, the claim on data `id`, as its healer or reclaimer would.
pub(crate) async fn claim_elsewhere(planter: &MetaPool, id: [u8; 16], take: bool) {
    let script = if take {
        "DEFINE SPACE IF NOT EXISTS heal_claims; SET heal_claims:$data = 'n9' IF ABSENT EXPIRE 60s;"
    } else {
        "SET heal_claims:$data = 'free' IF = 'n9' EXPIRE 1ms;"
    };
    planter
        .run(script, vec![("data".to_owned(), Value::Uuid(id))])
        .await
        .expect("claim statement");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_claim_left_under_this_nodes_own_name_does_not_hold_it_off() {
    let _why = IGNORED;
    let member = written().await;
    let id = data_id(&member).await;
    let path = shard_file(&member, 1);
    std::fs::remove_file(&path).expect("shard removed");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    // What a process of this node that died mid-heal leaves behind: one healer runs per process, so a claim under
    // this node's own name can only be a dead one.
    member
        .planter
        .run(
            "DEFINE SPACE IF NOT EXISTS heal_claims; SET heal_claims:$data = 'n1' IF ABSENT EXPIRE 60s;",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("claim left");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.held), (1, 1, 0));
    assert!(path.exists(), "rewritten");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_data_id_another_node_holds_is_left_to_it_and_healed_once_released() {
    let _why = IGNORED;
    let member = written().await;
    let id = data_id(&member).await;
    let path = shard_file(&member, 1);
    std::fs::remove_file(&path).expect("shard removed");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    claim_elsewhere(&member.planter, id, true).await;
    let before = crate::reclaim::store_now(&member.planter).await;
    let left = member.storage.objects().heal(100).await.expect("heal pass");
    let after = crate::reclaim::store_now(&member.planter).await;
    assert_eq!((left.examined, left.healed, left.held), (1, 0, 1));
    assert!(
        !path.exists(),
        "the shard was rewritten under another node's claim"
    );
    assert_eq!(
        count(&member.planter, "heal_queue").await,
        1,
        "still listed"
    );
    let due = crate::reclaim::due_at(&member.planter, "heal_queue", id).await;
    assert!(
        (before + retry(HEAL_RETRY_SECS)..=after + retry(HEAL_RETRY_SECS)).contains(&due),
        "due at {due}, not stepped aside for the holder from the pass between {before} and {after}"
    );
    claim_elsewhere(&member.planter, id, false).await;
    // The deferral passes.
    member
        .planter
        .run(
            "UPDATE heal_queue:$data SET due = time::now();",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("due moved to now");
    let healed = member.storage.objects().heal(100).await.expect("heal pass");
    assert_eq!((healed.examined, healed.healed, healed.at_risk), (1, 1, 0));
    assert!(path.exists(), "rewritten once the claim was handed back");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_data_id_being_healed_elsewhere_is_not_reclaimed_under_it() {
    let _why = IGNORED;
    let member = written().await;
    let id = data_id(&member).await;
    claim_elsewhere(&member.planter, id, true).await;
    let deleted = call(&member.state, "DELETE", "/shards/big", vec![], b"").await;
    assert_eq!(deleted.status, 204);
    let before = crate::reclaim::store_now(&member.planter).await;
    member
        .storage
        .objects()
        .reclaim(0, 100)
        .await
        .expect("reclaim pass");
    let after = crate::reclaim::store_now(&member.planter).await;
    assert!(
        shard_file_exists(&member, 0),
        "shards removed under another node's claim"
    );
    assert_eq!(count(&member.planter, "reclaims").await, 1, "still queued");
    let due = crate::reclaim::due_at(&member.planter, "reclaims", id).await;
    assert!(
        (before + retry(RECLAIM_RETRY_SECS)..=after + retry(RECLAIM_RETRY_SECS)).contains(&due),
        "due at {due}, not behind the entries waiting after it from the pass between {before} and {after}"
    );
    claim_elsewhere(&member.planter, id, false).await;
    // The deferral passes.
    member
        .planter
        .run(
            "UPDATE reclaims:$data SET due = time::now();",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("due moved to now");
    let reclaimed = member
        .storage
        .objects()
        .reclaim(0, 100)
        .await
        .expect("reclaim pass");
    assert_eq!(reclaimed.removed, 1);
    assert_eq!(count(&member.planter, "shard_sets").await, 0);
}

fn shard_file_exists(member: &Member, index: u16) -> bool {
    let suffix = format!(".s{index}");
    std::iter::once(&member.dir)
        .chain(member.peers.iter().map(|peer| &peer.dir))
        .any(|dir| has(dir, &suffix))
}

fn has(dir: &std::path::Path, suffix: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                has(&path, suffix)
            } else {
                path.to_string_lossy().ends_with(suffix)
            }
        })
    })
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn shard_bytes_whose_shard_set_is_gone_are_swept_and_live_ones_kept() {
    let _why = IGNORED;
    let member = written().await;
    // A second object, whose shard set stays: the control the sweep must not touch.
    let put = call(&member.state, "PUT", "/shards/kept", vec![], &striped()).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(shard_files(&member.dir), 2, "one shard of each object here");
    member
        .planter
        .run(
            "DELETE shard_sets:$data;",
            vec![("data".to_owned(), Value::Uuid(data_id(&member).await))],
        )
        .await
        .expect("shard set forgotten");
    let reclaimed = member
        .storage
        .objects()
        .reclaim(0, 100)
        .await
        .expect("reclaim pass");
    assert_eq!(
        shard_files(&member.dir),
        1,
        "the orphan swept, the live shard kept"
    );
    assert_eq!(reclaimed.orphans, 1);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_heal_that_keeps_failing_steps_aside_and_is_reported_at_risk() {
    let _why = IGNORED;
    let member = written().await;
    let id = data_id(&member).await;
    for index in [0, 3, 5] {
        std::fs::remove_file(shard_file(&member, index)).expect("shard removed");
    }
    let read = call(&member.state, "GET", "/shards/big", vec![], b"").await;
    assert_eq!(read.status, 503, "{}", read.body);
    let objects = member.storage.objects();
    for attempt in 1..=HEAL_AT_RISK_ATTEMPTS {
        let before = crate::reclaim::store_now(&member.planter).await;
        let healed = objects.heal(100).await.expect("heal pass");
        let after = crate::reclaim::store_now(&member.planter).await;
        assert_eq!(
            (healed.examined, healed.at_risk),
            (1, 1),
            "attempt {attempt}"
        );
        let due = crate::reclaim::due_at(&member.planter, "heal_queue", id).await;
        assert!(
            (before + retry(HEAL_RETRY_SECS)..=after + retry(HEAL_RETRY_SECS)).contains(&due),
            "attempt {attempt} due at {due}, not out of the head from the pass between {before} and {after}"
        );
        let early = objects.heal(100).await.expect("heal pass");
        assert_eq!(early.examined, 0, "a deferred heal waits its turn");
        // The deferral passes.
        member
            .planter
            .run(
                "UPDATE heal_queue:$data SET due = time::now();",
                vec![("data".to_owned(), Value::Uuid(id))],
            )
            .await
            .expect("due moved to now");
    }
    let backlog = objects
        .heal_backlog(100)
        .await
        .expect("backlog")
        .expect("a cluster member");
    assert_eq!((backlog.listed, backlog.at_risk), (1, 1));
}

/// A retry delay as the signed seconds the store's arithmetic answers.
fn retry(secs: u64) -> i64 {
    i64::try_from(secs).expect("a small constant")
}
