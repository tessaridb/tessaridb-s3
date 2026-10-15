//! Region failover under an epoch: an operator fails a region over to another, recorded in the metadata with a new
//! epoch; the failed-over region refuses client writes while still taking the promoted region's replicas (its way to
//! catch up), and a restore opens it again under a higher epoch.

use tessari_s3_storage::regions::{FailOver, Restore};

use crate::console::{send, sign_in};
use crate::replication::{ENABLED, replica_put, sequence_id};
use crate::{IGNORED, call, fresh_console};

const PROMOTED: &str = "eu-west-1";

fn view(seen: &crate::console::Seen) -> (u64, bool, Option<String>) {
    let json = seen.json();
    (
        json["epoch"].as_u64().expect("an epoch"),
        json["failed_over"].as_bool().expect("failed_over"),
        json["promoted"].as_str().map(str::to_owned),
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_failed_over_region_refuses_client_writes_takes_replicas_and_reopens_on_restore() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let region = state.region().to_owned();
    let token = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    let versioned = call(&state, "PUT", "/held", vec![("versioning", None)], ENABLED).await;
    assert_eq!(versioned.status, 200);
    let before = call(&state, "PUT", "/held/before", vec![], b"one").await;
    assert_eq!(before.status, 200);
    let etag = before
        .headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("an etag")
        .to_owned();

    let to_itself = format!(r#"{{"region":"{region}","promoted":"{region}","reason":"drill"}}"#);
    let refused = send(
        &console,
        "POST",
        "/api/v1/regions/failover",
        Some(&token),
        Some(&to_itself),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code().as_deref()),
        (400, Some("invalid_promotion"))
    );

    let body = format!(r#"{{"region":"{region}","promoted":"{PROMOTED}","reason":"drill"}}"#);
    let failed_over = send(
        &console,
        "POST",
        "/api/v1/regions/failover",
        Some(&token),
        Some(&body),
    )
    .await;
    assert_eq!(failed_over.status, 200, "{:?}", failed_over.json());
    assert_eq!(view(&failed_over), (1, true, Some(PROMOTED.to_owned())));
    let again = send(
        &console,
        "POST",
        "/api/v1/regions/failover",
        Some(&token),
        Some(&body),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (409, Some("region_failed_over"))
    );

    let listed = send(&console, "GET", "/api/v1/regions", Some(&token), None).await;
    assert_eq!(listed.status, 200);
    let regions = listed.json()["regions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        regions
            .iter()
            .any(|r| r["region"].as_str() == Some(region.as_str()) && r["epoch"] == 1),
        "{regions:?}"
    );

    let write = call(&state, "PUT", "/held/during", vec![], b"two").await;
    assert_eq!(write.status, 503, "{}", write.body);
    assert!(
        write.body.contains("ServiceUnavailable") && write.body.contains(PROMOTED),
        "{}",
        write.body
    );
    let delete = call(&state, "DELETE", "/held/before", vec![], b"").await;
    assert_eq!(delete.status, 503, "a delete is a write too");
    let read = call(&state, "GET", "/held/before", vec![], b"").await;
    assert_eq!(
        (read.status, read.body.as_str()),
        (200, "one"),
        "reads go on"
    );
    let replica = replica_put(
        &state,
        "/held/replica",
        &sequence_id(7, 1),
        b"from the promoted region",
    )
    .await;
    assert_eq!(
        replica.status, 200,
        "the promoted region's replica is taken: {}",
        replica.body
    );

    let created = send(
        &console,
        "POST",
        "/api/v1/buckets",
        Some(&token),
        Some(r#"{"name":"another","reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (created.status, created.code().as_deref()),
        (503, Some("region_failed_over"))
    );
    let removal = format!(
        r#"{{"etag":{},"reason":"r"}}"#,
        serde_json::to_string(&etag).expect("json")
    );
    let removed = send(
        &console,
        "DELETE",
        "/api/v1/buckets/held/object?key=before",
        Some(&token),
        Some(&removal),
    )
    .await;
    assert_eq!(
        (removed.status, removed.code().as_deref()),
        (503, Some("region_failed_over"))
    );
    let dropped = send(
        &console,
        "DELETE",
        "/api/v1/buckets/held",
        Some(&token),
        Some(r#"{"reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (dropped.status, dropped.code().as_deref()),
        (503, Some("region_failed_over"))
    );

    let back = format!(r#"{{"region":"{region}","reason":"recovered"}}"#);
    let restored = send(
        &console,
        "POST",
        "/api/v1/regions/restore",
        Some(&token),
        Some(&back),
    )
    .await;
    assert_eq!(restored.status, 200, "{:?}", restored.json());
    assert_eq!(view(&restored), (2, false, None));
    let twice = send(
        &console,
        "POST",
        "/api/v1/regions/restore",
        Some(&token),
        Some(&back),
    )
    .await;
    assert_eq!(
        (twice.status, twice.code().as_deref()),
        (409, Some("region_not_failed_over"))
    );
    assert_eq!(
        call(&state, "PUT", "/held/after", vec![], b"three")
            .await
            .status,
        200
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn two_operators_failing_a_region_over_at_once_never_both_succeed() {
    let _why = IGNORED;
    let (state, _) = crate::fresh().await;
    let regions = state.storage().regions();
    let mut both = 0_u32;
    for round in 0..20_u64 {
        let (first, second) = tokio::join!(
            regions.fail_over("us-east-1", PROMOTED, "root", "first"),
            regions.fail_over("us-east-1", "ap-south-1", "root", "second"),
        );
        let won = [first.expect("answered"), second.expect("answered")]
            .iter()
            .filter(|answer| matches!(answer, FailOver::FailedOver(_)))
            .count();
        assert!(won >= 1, "round {round}: nobody won");
        both = both.saturating_add(u32::from(won > 1));
        let restored = regions.restore("us-east-1", "root", "next round").await;
        let Ok(Restore::Restored(region)) = restored else {
            panic!("round {round}: not restored: {restored:?}");
        };
        assert_eq!(
            region.epoch,
            round.saturating_mul(2).saturating_add(2),
            "one change per failover and per restore"
        );
    }
    assert_eq!(
        both, 0,
        "rounds in which both operators failed the region over"
    );
}
