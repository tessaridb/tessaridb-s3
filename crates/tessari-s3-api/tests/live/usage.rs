//! Usage measured against a real node: the pass counts each bucket's current objects and their logical bytes, one
//! member measures at a time, and a bucket emptied since the last pass leaves the record.

use tessari_s3_storage::usage::BucketUsage;

use serde_json::json;

use crate::console::{send, sign_in};
use crate::{IGNORED, call, fresh, fresh_console};

fn figures(bucket: &str, objects: u64, bytes: u64) -> BucketUsage {
    BucketUsage {
        bucket: bucket.to_owned(),
        objects,
        bytes,
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_pass_counts_each_buckets_current_objects_and_bytes() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let usage = state.storage().usage();
    assert_eq!(
        usage.latest().await.expect("read"),
        None,
        "nothing measured yet"
    );
    for bucket in ["photos", "docs", "empty"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    for (path, body) in [
        ("/photos/a", &b"12345"[..]),
        ("/photos/b", &b"1234567890"[..]),
        ("/photos/c", &b""[..]),
        ("/docs/readme", &b"1234567"[..]),
        // An overwrite counts once, at its new size.
        ("/photos/b", &b"1234"[..]),
    ] {
        assert_eq!(
            call(&state, "PUT", path, vec![], body).await.status,
            200,
            "{path}"
        );
    }
    assert_eq!(usage.refresh("member-1").await.expect("measured"), Some(2));
    assert_eq!(
        usage.refresh("member-2").await.expect("asked"),
        None,
        "member-1 holds the pass"
    );
    let first = usage.latest().await.expect("read").expect("a measurement");
    assert_eq!(
        first.buckets,
        vec![figures("docs", 1, 7), figures("photos", 3, 9)]
    );
    assert_eq!(
        call(&state, "DELETE", "/docs/readme", vec![], b"")
            .await
            .status,
        204
    );
    assert_eq!(usage.refresh("member-1").await.expect("measured"), Some(1));
    let second = usage.latest().await.expect("read").expect("a measurement");
    assert_eq!(second.buckets, vec![figures("photos", 3, 9)]);
    assert!(second.taken >= first.taken, "stamped by the store's clock");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_console_reports_the_last_measurement_and_this_nodes_drive() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let token = sign_in(&console).await;
    let before = send(&console, "GET", "/api/v1/usage", Some(&token), None).await;
    assert_eq!(before.status, 200);
    assert_eq!(
        before.json(),
        json!({"taken": null, "buckets": [], "objects": 0, "bytes": 0}),
        "nothing measured yet"
    );
    assert_eq!(call(&state, "PUT", "/media", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/media/clip", vec![], b"0123456789")
            .await
            .status,
        200
    );
    state
        .storage()
        .usage()
        .refresh("standalone")
        .await
        .expect("measured");
    let after = send(&console, "GET", "/api/v1/usage", Some(&token), None).await;
    let body = after.json();
    assert_eq!(
        (&body["buckets"], &body["objects"], &body["bytes"]),
        (
            &json!([{"bucket": "media", "objects": 1, "bytes": 10}]),
            &json!(1),
            &json!(10)
        )
    );
    assert!(
        body["taken"]
            .as_str()
            .is_some_and(|taken| taken.ends_with('Z')),
        "{body}"
    );
    let status = send(&console, "GET", "/api/v1/status", Some(&token), None).await;
    assert_eq!(
        status.json()["drive"],
        serde_json::Value::Null,
        "this node has no data directory"
    );
}
