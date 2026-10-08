//! A bucket quota is checked against a recent usage measurement, against a real node: once the last measurement is
//! older than two usage intervals (or was never taken), a write to a LIMITED bucket counts that one bucket on the write
//! path instead of trusting a figure nobody refreshed, while a bucket with no limit is unaffected. Each step is chosen
//! so the stored figure and the live count give different answers. The clock is moved, not waited for.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tessari_s3_api::ApiState;
use tessari_s3_core::quota::Quota;
use tessari_s3_types::BucketName;

use crate::console::{send, sign_in};
use crate::console_users::states;
use crate::{IGNORED, Seen, call};

/// Two intervals of the live suite's 60-second default.
const WINDOW_SECS: i64 = 120;

/// Measures usage and sets the test's clock to the measurement's own time: the store stamps it by its clock, and a
/// second boundary falling between the two clocks would otherwise move the window's edge by one.
async fn measure(state: &ApiState, now: &Arc<AtomicI64>) {
    let usage = state.storage().usage();
    usage.refresh("standalone").await.expect("measured");
    let taken = usage
        .latest()
        .await
        .expect("read")
        .expect("a measurement")
        .taken;
    now.store(taken.seconds, Ordering::Release);
}

fn over_quota(seen: &Seen) {
    assert_eq!(
        (seen.status, seen.code.as_deref()),
        (400, Some("InvalidRequest")),
        "{}",
        seen.body
    );
    assert!(seen.body.contains("quota"), "{}", seen.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_limited_bucket_is_counted_live_once_its_measurement_is_older_than_the_window() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let root = sign_in(&console).await;
    let stale_shown = async || {
        send(&console, "GET", "/api/v1/usage", Some(&root), None)
            .await
            .json()["stale"]
            .clone()
    };
    for bucket in ["/held", "/open"] {
        assert_eq!(call(&state, "PUT", bucket, vec![], b"").await.status, 200);
    }
    let held = BucketName::new("held").expect("a name");
    let limit = Quota {
        max_bytes: Some(10),
        max_objects: None,
    };
    assert!(
        state
            .storage()
            .buckets()
            .set_quota(&held, limit)
            .await
            .expect("set")
    );

    // Never measured: the bucket is counted as it stands, so the second write sees the first.
    assert_eq!(stale_shown().await, serde_json::Value::Bool(true));
    assert_eq!(
        call(&state, "PUT", "/held/a", vec![], b"123456")
            .await
            .status,
        200
    );
    over_quota(&call(&state, "PUT", "/held/b", vec![], b"123456").await);

    // Measured (6 bytes): writes are checked against the stored figure.
    measure(&state, &now).await;
    assert_eq!(stale_shown().await, serde_json::Value::Bool(false));
    assert_eq!(
        call(&state, "PUT", "/held/c", vec![], b"123").await.status,
        200
    );
    // At the window's edge the stored figure still counts: 6 + 4 fits, though the bucket now holds 9.
    now.fetch_add(WINDOW_SECS, Ordering::AcqRel);
    assert_eq!(
        call(&state, "PUT", "/held/d", vec![], b"1234").await.status,
        200
    );

    // Past it the bucket is counted again: 13 held, so one byte more passes the limit the stored 6 would not.
    now.fetch_add(1, Ordering::AcqRel);
    assert_eq!(
        stale_shown().await,
        serde_json::Value::Bool(true),
        "the console shows an operator the scanner has fallen behind"
    );
    over_quota(&call(&state, "PUT", "/held/e", vec![], b"1").await);
    let open = call(&state, "PUT", "/open/e", vec![], b"no limit").await;
    assert_eq!(
        open.status, 200,
        "a bucket without a limit is not counted against anything"
    );
}
