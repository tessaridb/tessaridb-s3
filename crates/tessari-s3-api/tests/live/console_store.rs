//! What the console stands on in storage: the action record answers newest first a page at a time with every field
//! as recorded and the store's clock; an object is deleted under its ETag only while the ETag still matches; and the
//! heal backlog is counted on a cluster member and absent off one.

use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::objects::{HealBacklog, RemovedIf};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::erasure_reads::{read_back, shard_file, striped, written};
use crate::{IGNORED, call, fresh};

fn action(n: u32, reason: Option<&str>) -> NewAction {
    NewAction {
        operator: "AKOPERATOR".to_owned(),
        operation: "delete_object".to_owned(),
        target: format!("b/k{n}"),
        reason: reason.map(str::to_owned),
        outcome: "done".to_owned(),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_action_record_answers_newest_first_a_page_at_a_time() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let actions = state.storage().actions();
    assert!(
        actions.recent(None, 10).await.expect("empty").is_empty(),
        "nothing recorded yet"
    );
    for n in 1..=5 {
        let reason = (n % 2 == 1).then_some("ticket 42");
        actions.record(&action(n, reason)).await.expect("recorded");
    }
    let newest = actions.recent(None, 2).await.expect("newest page");
    let positions: Vec<u64> = newest.iter().map(|a| a.position).collect();
    assert_eq!(positions, vec![5, 4]);
    let five = newest.first().expect("one");
    assert_eq!(
        (
            five.operator.as_str(),
            five.target.as_str(),
            five.reason.as_deref(),
            five.outcome.as_str()
        ),
        ("AKOPERATOR", "b/k5", Some("ticket 42"), "done")
    );
    assert_eq!(newest.get(1).map(|a| a.reason.clone()), Some(None));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs();
    let at = u64::try_from(five.at.seconds).expect("after the epoch");
    assert!(at.abs_diff(now) < 120, "the store's clock: {at} vs {now}");
    let older = actions.recent(Some(4), 10).await.expect("older page");
    let positions: Vec<u64> = older.iter().map(|a| a.position).collect();
    assert_eq!(positions, vec![3, 2, 1]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_is_deleted_under_its_etag_only_while_the_etag_matches() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cons", vec![], b"").await.status, 200);
    let put = call(&state, "PUT", "/cons/doc", vec![], b"first").await;
    let etag = put
        .headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("etag")
        .to_owned();
    let objects = state.storage().objects();
    let bucket = BucketName::new("cons").expect("bucket");
    let key = ObjectKey::new("doc").expect("key");
    let stale = objects
        .delete_if_match(&bucket, &key, "\"0123456789abcdef0123456789abcdef\"")
        .await
        .expect("stale etag");
    assert_eq!(stale, RemovedIf::PreconditionFailed);
    assert_eq!(
        call(&state, "GET", "/cons/doc", vec![], b"").await.status,
        200,
        "kept"
    );
    let removed = objects
        .delete_if_match(&bucket, &key, &etag)
        .await
        .expect("matching etag");
    assert_eq!(removed, RemovedIf::Done);
    assert_eq!(
        call(&state, "GET", "/cons/doc", vec![], b"").await.status,
        404,
        "gone"
    );
    let again = objects.delete_if_match(&bucket, &key, &etag).await;
    assert_eq!(again.expect("again"), RemovedIf::NoSuchKey);
    let elsewhere = BucketName::new("no-such-bucket").expect("bucket");
    let missing = objects.delete_if_match(&elsewhere, &key, &etag).await;
    assert_eq!(missing.expect("no bucket"), RemovedIf::NoSuchBucket);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_heal_backlog_is_counted_on_a_member_and_absent_off_one() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let off = state.storage().objects().heal_backlog(10).await;
    assert_eq!(off.expect("off a cluster"), None);
    let member = written().await;
    let objects = member.storage.objects();
    let whole = objects.heal_backlog(10).await.expect("nothing listed");
    assert_eq!(
        whole,
        Some(HealBacklog {
            listed: 0,
            more: false
        })
    );
    std::fs::remove_file(shard_file(&member, 1)).expect("shard removed");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    let listed = objects.heal_backlog(10).await.expect("one listed");
    assert_eq!(
        listed,
        Some(HealBacklog {
            listed: 1,
            more: false
        })
    );
    let bounded = objects.heal_backlog(0).await.expect("bounded");
    assert_eq!(
        bounded,
        Some(HealBacklog {
            listed: 0,
            more: true
        })
    );
}
