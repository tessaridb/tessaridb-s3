//! Usage measured against a real node: the pass counts each bucket's current objects and their logical bytes, one
//! member measures at a time, and a bucket emptied since the last pass leaves the record.

use tessari_s3_storage::usage::BucketUsage;

use serde_json::json;

use crate::console::{send, sign_in};
use crate::multipart;
use crate::{IGNORED, call, fresh, fresh_console};

/// A bucket's figures when every object is small enough to be held inline: nothing on the drives.
fn figures(bucket: &str, objects: u64, bytes: u64) -> BucketUsage {
    BucketUsage {
        bucket: bucket.to_owned(),
        objects,
        bytes,
        inline_bytes: bytes,
        raw_bytes: 0,
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
        json!({"taken": null, "buckets": [], "objects": 0, "bytes": 0, "inline_bytes": 0, "raw_bytes": 0}),
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
        (
            &body["buckets"],
            &body["objects"],
            &body["bytes"],
            &body["inline_bytes"],
            &body["raw_bytes"]
        ),
        (
            &json!([{"bucket": "media", "objects": 1, "bytes": 10, "inline_bytes": 10, "raw_bytes": 0}]),
            &json!(1),
            &json!(10),
            &json!(10),
            &json!(0)
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

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_pass_counts_inline_bytes_and_the_bytes_on_the_drive() {
    const MIB: usize = 1 << 20;
    let _why = IGNORED;
    let (state, _) = fresh().await;
    for path in ["/media", "/open"] {
        assert_eq!(call(&state, "PUT", path, vec![], b"").await.status, 200);
    }
    let big = vec![7_u8; MIB + 1];
    for (path, body) in [("/media/small", &b"0123456789"[..]), ("/media/big", &big)] {
        assert_eq!(
            call(&state, "PUT", path, vec![], body).await.status,
            200,
            "{path}"
        );
    }
    // A completed multipart object of two parts, and an open upload holding one part.
    let part = vec![1_u8; 5 * MIB];
    let id = multipart::create(&state, "/media/joined").await;
    let first = multipart::etag(&multipart::part(&state, "/media/joined", &id, "1", &part).await);
    let second =
        multipart::etag(&multipart::part(&state, "/media/joined", &id, "2", b"tail").await);
    let listed = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{first}</ETag></Part>\
         <Part><PartNumber>2</PartNumber><ETag>{second}</ETag></Part></CompleteMultipartUpload>"
    );
    let completed = call(
        &state,
        "POST",
        "/media/joined",
        vec![("uploadId", Some(&id))],
        listed.as_bytes(),
    )
    .await;
    assert_eq!(completed.status, 200, "{}", completed.body);
    let open = multipart::create(&state, "/open/pending").await;
    assert_eq!(
        multipart::part(&state, "/open/pending", &open, "1", b"waiting")
            .await
            .status,
        200
    );
    state
        .storage()
        .usage()
        .refresh("member-1")
        .await
        .expect("measured");

    let measured = state
        .storage()
        .usage()
        .latest()
        .await
        .expect("read")
        .expect("taken");
    let joined = 5 * MIB + 4;
    let to = |n: usize| u64::try_from(n).expect("a size");
    assert_eq!(
        measured.buckets,
        vec![
            BucketUsage {
                bucket: "media".to_owned(),
                objects: 3,
                bytes: to(10 + MIB + 1 + joined),
                inline_bytes: 10,
                raw_bytes: to(MIB + 1 + joined),
            },
            BucketUsage {
                bucket: "open".to_owned(),
                objects: 0,
                bytes: 0,
                inline_bytes: 0,
                raw_bytes: 7,
            },
        ],
        "a node on its own holds data files as they are; an open upload's part is on the drive too"
    );
}
