//! Bucket event notifications against a real node (S3 ref 11 M5): every change to an object in a bucket that has a
//! notification configuration leaves one record in the `object_events` outbox, written in the change's own commit and
//! named by what the request did; a bucket without one records nothing.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::complete::multipart_put;
use crate::{IGNORED, call, call_with, fresh};

/// The event names in the outbox for `key`, sorted.
async fn recorded(planter: &MetaPool, key: &str) -> Vec<String> {
    let answers = planter
        .run(
            "SELECT event FROM object_events WHERE key = $key;",
            vec![("key".to_owned(), Value::String(key.to_owned()))],
        )
        .await
        .expect("outbox read");
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Vec::new();
    };
    let mut names: Vec<String> = records
        .into_iter()
        .map(|(_, record)| match record {
            Value::Object(fields) => match fields.get("event") {
                Some(Value::String(name)) => name.clone(),
                other => panic!("an event name, not {other:?}"),
            },
            other => panic!("an outbox record, not {other:?}"),
        })
        .collect();
    names.sort();
    names
}

/// Gives `bucket` a notification configuration, as the configuration API will.
async fn configure(planter: &MetaPool, bucket: &str) {
    planter
        .run(
            "UPDATE buckets:$name SET notify = { rules: [] };",
            vec![("name".to_owned(), Value::String(bucket.to_owned()))],
        )
        .await
        .expect("configured");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_change_in_a_configured_bucket_leaves_its_event_and_others_leave_none() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for name in ["notified", "quiet"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    configure(&planter, "notified").await;
    for bucket in ["notified", "quiet"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}/k"), vec![], b"one")
                .await
                .status,
            200
        );
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}/k"), vec![], b"two")
                .await
                .status,
            200
        );
        let source = format!("/{bucket}/k");
        let copied = call_with(
            &state,
            "PUT",
            &format!("/{bucket}/copy"),
            vec![],
            vec![("x-amz-copy-source", source.as_str())],
            b"",
        )
        .await;
        assert_eq!(copied.status, 200, "{}", copied.body);
        multipart_put(&state, &format!("/{bucket}/big"), &[b"parts"]).await;
        assert_eq!(
            call(&state, "DELETE", &format!("/{bucket}/k"), vec![], b"")
                .await
                .status,
            204
        );
    }
    assert_eq!(
        recorded(&planter, "k").await,
        vec![
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectRemoved:Delete".to_owned(),
        ],
        "two puts and a delete of `k` in the configured bucket, nothing from the quiet one"
    );
    assert_eq!(
        recorded(&planter, "copy").await,
        vec!["s3:ObjectCreated:Copy".to_owned()]
    );
    assert_eq!(
        recorded(&planter, "big").await,
        vec!["s3:ObjectCreated:CompleteMultipartUpload".to_owned()]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_refused_write_leaves_no_event_and_concurrent_writers_all_record_theirs() {
    use crate::versions_kept::set_versioning;
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/marked", vec![], b"").await.status,
        200
    );
    set_versioning(&state, "marked", "Enabled").await;
    configure(&planter, "marked").await;
    assert_eq!(
        call(&state, "PUT", "/marked/k", vec![], b"one")
            .await
            .status,
        200
    );
    // A write its own condition refuses commits nothing, so it records nothing.
    let refused = call_with(
        &state,
        "PUT",
        "/marked/k",
        vec![],
        vec![("if-none-match", "*")],
        b"two",
    )
    .await;
    assert_eq!(refused.status, 412);
    assert_eq!(
        call(&state, "DELETE", "/marked/k", vec![], b"")
            .await
            .status,
        204
    );
    assert_eq!(
        recorded(&planter, "k").await,
        vec![
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectRemoved:DeleteMarkerCreated".to_owned()
        ],
        "the put and the marker, nothing for the refused write"
    );
    // Writers of different keys at once: every write commits and leaves its event — the outbox takes no shared
    // identity, so it costs the writers no refusals.
    let writes = (0..16).map(|n| {
        let state = state.clone();
        async move {
            call(
                &state,
                "PUT",
                &format!("/marked/many-{n:02}"),
                vec![],
                b"bytes",
            )
            .await
            .status
        }
    });
    let statuses = futures_util::future::join_all(writes).await;
    assert!(statuses.iter().all(|status| *status == 200), "{statuses:?}");
    for n in 0..16 {
        assert_eq!(
            recorded(&planter, &format!("many-{n:02}")).await,
            vec!["s3:ObjectCreated:Put".to_owned()]
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_removal_follows_the_configuration_at_the_time_of_the_removal() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/later", vec![], b"").await.status, 200);
    assert_eq!(
        call(
            &state,
            "PUT",
            "/later/before",
            vec![],
            b"written unconfigured"
        )
        .await
        .status,
        200
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/later/after",
            vec![],
            b"written unconfigured"
        )
        .await
        .status,
        200
    );
    configure(&planter, "later").await;
    assert_eq!(
        call(&state, "DELETE", "/later/before", vec![], b"")
            .await
            .status,
        204
    );
    assert_eq!(
        recorded(&planter, "before").await,
        vec!["s3:ObjectRemoved:Delete".to_owned()]
    );
    // And the other way: configured when written, unconfigured when removed.
    assert_eq!(
        call(&state, "PUT", "/later/gone", vec![], b"written configured")
            .await
            .status,
        200
    );
    planter
        .run(
            "UPDATE buckets:$name SET notify = NONE;",
            vec![("name".to_owned(), Value::String("later".to_owned()))],
        )
        .await
        .expect("unconfigured");
    assert_eq!(
        call(&state, "DELETE", "/later/gone", vec![], b"")
            .await
            .status,
        204
    );
    assert_eq!(
        recorded(&planter, "gone").await,
        vec!["s3:ObjectCreated:Put".to_owned()]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_or_marker_removed_by_id_records_one_permanent_removal() {
    use crate::versions_delete::delete_version;
    use crate::versions_kept::{put, set_versioning, version_header};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/kept", vec![], b"").await.status, 200);
    set_versioning(&state, "kept", "Enabled").await;
    configure(&planter, "kept").await;
    let one = version_header(&put(&state, "/kept/k", b"one").await).expect("an id");
    let two = version_header(&put(&state, "/kept/k", b"two").await).expect("an id");
    // A noncurrent version, then the current one — the key's last — each removed for good.
    assert_eq!(delete_version(&state, "/kept/k", &one).await.status, 204);
    assert_eq!(delete_version(&state, "/kept/k", &two).await.status, 204);
    // A marker over `three`, removed by its id: `three` is current again, and that is no new object.
    put(&state, "/kept/k", b"three").await;
    let deleted = call(&state, "DELETE", "/kept/k", vec![], b"").await;
    assert_eq!(deleted.status, 204);
    let marker = version_header(&deleted).expect("the marker's id");
    assert_eq!(delete_version(&state, "/kept/k", &marker).await.status, 204);
    assert_eq!(
        recorded(&planter, "k").await,
        vec![
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectRemoved:Delete".to_owned(),
            "s3:ObjectRemoved:Delete".to_owned(),
            "s3:ObjectRemoved:Delete".to_owned(),
            "s3:ObjectRemoved:DeleteMarkerCreated".to_owned(),
        ],
        "three puts, the marker the delete added, and one permanent removal per version id"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn what_lifecycle_removes_is_named_a_lifecycle_expiration() {
    use crate::lifecycle::{days_ahead, lifecycle};
    use crate::versions_kept::{put, set_versioning};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    set_versioning(&state, "cycle", "Enabled").await;
    configure(&planter, "cycle").await;
    put(&state, "/cycle/logs/a", b"first").await;
    put(&state, "/cycle/logs/a", b"second").await;
    let rules = "<LifecycleConfiguration><Rule><ID>logs</ID><Filter><Prefix>logs/</Prefix></Filter>\
        <Status>Enabled</Status><Expiration><Days>1</Days></Expiration>\
        <NoncurrentVersionExpiration><NoncurrentDays>1</NoncurrentDays></NoncurrentVersionExpiration>\
        </Rule></LifecycleConfiguration>";
    assert_eq!(lifecycle(&state, "PUT", rules).await.status, 200);
    let passes = state.storage().lifecycle();
    let now = days_ahead(3);
    // Pass 1 removes the noncurrent `first` and expires `second` behind a marker.
    passes.pass(now, 100, 0).await.expect("pass 1");
    assert_eq!(
        recorded(&planter, "logs/a").await,
        vec![
            "s3:LifecycleExpiration:Delete".to_owned(),
            "s3:LifecycleExpiration:DeleteMarkerCreated".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
        ]
    );
    // Passes 2 and 3 remove `second`, now noncurrent, then the marker left alone.
    passes.pass(now, 100, 0).await.expect("pass 2");
    passes.pass(now, 100, 0).await.expect("pass 3");
    assert_eq!(
        recorded(&planter, "logs/a").await,
        vec![
            "s3:LifecycleExpiration:Delete".to_owned(),
            "s3:LifecycleExpiration:Delete".to_owned(),
            "s3:LifecycleExpiration:Delete".to_owned(),
            "s3:LifecycleExpiration:DeleteMarkerCreated".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
            "s3:ObjectCreated:Put".to_owned(),
        ]
    );
}
