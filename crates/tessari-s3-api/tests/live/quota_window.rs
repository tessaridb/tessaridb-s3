//! A bucket quota is checked only against a recent usage measurement, against a real node: once the last measurement
//! is older than two usage intervals, a write to a LIMITED bucket is refused as unavailable rather than admitted on a
//! figure nobody refreshed, while a bucket with no limit is unaffected; a new measurement lets the write through. The
//! clock is moved, not waited for.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tessari_s3_api::ApiState;
use tessari_s3_core::quota::Quota;
use tessari_s3_types::BucketName;

use crate::console::{send, sign_in};
use crate::console_users::states;
use crate::{IGNORED, call};

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

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_limited_bucket_refuses_writes_once_its_measurement_is_older_than_the_window() {
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
        max_bytes: Some(1 << 20),
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
    // Never measured: nothing to check the limit against.
    let unmeasured = call(&state, "PUT", "/held/a", vec![], b"unmeasured").await;
    assert_eq!(
        (unmeasured.status, unmeasured.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        unmeasured.body
    );
    measure(&state, &now).await;
    assert_eq!(
        call(&state, "PUT", "/held/a", vec![], b"fresh")
            .await
            .status,
        200
    );

    // At the window's edge the measurement still counts.
    now.fetch_add(WINDOW_SECS, Ordering::AcqRel);
    assert_eq!(
        call(&state, "PUT", "/held/b", vec![], b"edge").await.status,
        200
    );
    assert_eq!(stale_shown().await, serde_json::Value::Bool(false));
    now.fetch_add(1, Ordering::AcqRel);
    let stale = call(&state, "PUT", "/held/c", vec![], b"stale").await;
    assert_eq!(
        (stale.status, stale.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        stale.body
    );
    assert!(
        stale.body.contains("usage"),
        "the refusal says why: {}",
        stale.body
    );
    assert_eq!(
        stale_shown().await,
        serde_json::Value::Bool(true),
        "the console shows an operator why"
    );
    let open = call(&state, "PUT", "/open/c", vec![], b"no limit").await;
    assert_eq!(
        open.status, 200,
        "a bucket without a limit is not measured against anything"
    );

    // A new measurement, and the clock with it.
    measure(&state, &now).await;
    assert_eq!(
        call(&state, "PUT", "/held/c", vec![], b"measured")
            .await
            .status,
        200
    );
}
