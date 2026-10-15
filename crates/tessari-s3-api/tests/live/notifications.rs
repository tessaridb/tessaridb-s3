//! Bucket event notifications against a real node (S3 ref 11 M5): every change to an object in a bucket that has a
//! notification configuration leaves one record in the `object_events` outbox, written in the change's own commit and
//! named by what the request did; a bucket without one records nothing.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::complete::multipart_put;
use crate::{IGNORED, call, call_with, fresh};

/// The event names in the outbox for `key`, sorted.
pub(crate) async fn recorded(planter: &MetaPool, key: &str) -> Vec<String> {
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
            &format!(
                "UPDATE {}.buckets:$name SET notify = {{ rules: [] }};",
                planter.global_database()
            ),
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
            &format!(
                "UPDATE {}.buckets:$name SET notify = NONE;",
                planter.global_database()
            ),
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

pub(crate) async fn notification(
    state: &tessari_s3_api::ApiState,
    method: &str,
    bucket: &str,
    body: &str,
) -> crate::Seen {
    call(
        state,
        method,
        &format!("/{bucket}"),
        vec![("notification", None)],
        body.as_bytes(),
    )
    .await
}

pub(crate) const CONFIGURATION: &str = "<NotificationConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
    <QueueConfiguration><Id>created-logs</Id><Queue>arn:tessaridb:sqs:::hooks</Queue>\
    <Event>s3:ObjectCreated:*</Event><Filter><S3Key><FilterRule><Name>prefix</Name><Value>logs/</Value></FilterRule>\
    <FilterRule><Name>Suffix</Name><Value>.txt</Value></FilterRule></S3Key></Filter></QueueConfiguration>\
    <QueueConfiguration><Queue>arn:tessaridb:sqs:::hooks</Queue><Event>s3:ObjectRemoved:Delete</Event>\
    <Event>s3:LifecycleExpiration:*</Event></QueueConfiguration></NotificationConfiguration>";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_notification_configuration_is_stored_read_back_and_turned_off() {
    use crate::multipart::text;
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/told", vec![], b"").await.status, 200);
    let none = notification(&state, "GET", "told", "").await;
    assert_eq!(none.status, 200, "{}", none.body);
    assert!(
        none.body.contains("NotificationConfiguration")
            && !none.body.contains("<QueueConfiguration>"),
        "an empty configuration: {}",
        none.body
    );
    let put = notification(&state, "PUT", "told", CONFIGURATION).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let got = notification(&state, "GET", "told", "").await;
    assert_eq!(got.status, 200, "{}", got.body);
    for fragment in [
        "<QueueConfiguration><Id>created-logs</Id><Queue>arn:tessaridb:sqs:::hooks</Queue>\
         <Event>s3:ObjectCreated:*</Event><Filter><S3Key><FilterRule><Name>prefix</Name><Value>logs/</Value>\
         </FilterRule><FilterRule><Name>suffix</Name><Value>.txt</Value></FilterRule></S3Key></Filter>\
         </QueueConfiguration>",
        "<Queue>arn:tessaridb:sqs:::hooks</Queue><Event>s3:ObjectRemoved:Delete</Event>\
         <Event>s3:LifecycleExpiration:*</Event></QueueConfiguration>",
    ] {
        assert!(got.body.contains(fragment), "{fragment} in {}", got.body);
    }
    let ids: Vec<String> = crate::multipart::texts(&got.body, "QueueConfiguration", "Id");
    assert_eq!(ids.len(), 2, "{}", got.body);
    assert!(!ids[1].is_empty(), "a rule sent with no Id is given one");
    // The configuration set through the API is what a write reads.
    call(&state, "PUT", "/told/logs/a.txt", vec![], b"bytes").await;
    assert_eq!(
        recorded(&planter, "logs/a.txt").await,
        vec!["s3:ObjectCreated:Put".to_owned()]
    );
    // An empty configuration turns notifications off.
    let off = notification(&state, "PUT", "told", "<NotificationConfiguration/>").await;
    assert_eq!(off.status, 200, "{}", off.body);
    assert!(
        !notification(&state, "GET", "told", "")
            .await
            .body
            .contains("<QueueConfiguration>")
    );
    call(&state, "PUT", "/told/logs/b.txt", vec![], b"bytes").await;
    assert!(recorded(&planter, "logs/b.txt").await.is_empty());
    assert_eq!(text(&none.body, "Code"), None);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn what_a_notification_configuration_may_not_say_is_refused_and_nothing_is_stored() {
    use crate::multipart::text;
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/strict", vec![], b"").await.status,
        200
    );
    let queue = |arn: &str, events: &str, filter: &str| {
        format!("<QueueConfiguration><Queue>{arn}</Queue>{events}{filter}</QueueConfiguration>")
    };
    let hooks = "arn:tessaridb:sqs:::hooks";
    let created = "<Event>s3:ObjectCreated:Put</Event>";
    let cases = [
        (queue("arn:tessaridb:sqs:::nowhere", created, ""), 400, "InvalidArgument"),
        (queue("arn:aws:sqs:us-east-1:123456789012:q", created, ""), 400, "InvalidArgument"),
        (queue(hooks, "<Event>s3:ObjectRestore:Post</Event>", ""), 400, "InvalidArgument"),
        (queue(hooks, "", ""), 400, "MalformedXML"),
        (
            format!("{}{}", queue(hooks, created, ""), queue(hooks, "<Event>s3:ObjectCreated:*</Event>", "")),
            400,
            "InvalidArgument",
        ),
        (
            queue(
                hooks,
                created,
                "<Filter><S3Key><FilterRule><Name>middle</Name><Value>x</Value></FilterRule></S3Key></Filter>",
            ),
            400,
            "InvalidArgument",
        ),
        (
            "<TopicConfiguration><Topic>arn:aws:sns:us-east-1:1:t</Topic><Event>s3:ObjectCreated:Put</Event>\
             </TopicConfiguration>"
                .to_owned(),
            501,
            "NotImplemented",
        ),
        (
            "<CloudFunctionConfiguration><CloudFunction>arn:aws:lambda:x</CloudFunction>\
             <Event>s3:ObjectCreated:Put</Event></CloudFunctionConfiguration>"
                .to_owned(),
            501,
            "NotImplemented",
        ),
        ("<EventBridgeConfiguration/>".to_owned(), 501, "NotImplemented"),
    ];
    for (inner, status, code) in cases {
        let document = format!("<NotificationConfiguration>{inner}</NotificationConfiguration>");
        let seen = notification(&state, "PUT", "strict", &document).await;
        assert_eq!(seen.status, status, "{document}: {}", seen.body);
        assert_eq!(
            text(&seen.body, "Code").as_deref(),
            Some(code),
            "{document}"
        );
    }
    let malformed = notification(&state, "PUT", "strict", "<NotificationConfiguration>").await;
    assert_eq!(
        text(&malformed.body, "Code").as_deref(),
        Some("MalformedXML")
    );
    assert!(
        !notification(&state, "GET", "strict", "")
            .await
            .body
            .contains("<QueueConfiguration>"),
        "nothing refused was stored"
    );
    assert_eq!(notification(&state, "GET", "absent", "").await.status, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn only_who_configures_the_bucket_sets_its_notifications_and_a_reader_reads_them() {
    use crate::access::{grant, user_with_key};
    use tessari_s3_core::authz::{Access, Role};
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let as_user = |pair, method: &'static str, body: &'static str| {
        let state = state.clone();
        async move {
            crate::call_as(
                &state,
                pair,
                method,
                "/shared",
                vec![("notification", None)],
                vec![],
                body.as_bytes(),
            )
            .await
            .status
        }
    };
    assert_eq!(
        crate::call_as(&state, ann.pair(), "PUT", "/shared", vec![], vec![], b"")
            .await
            .status,
        200
    );
    grant(
        storage,
        "bob",
        "shared",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    assert_eq!(
        as_user(bob.pair(), "PUT", CONFIGURATION).await,
        403,
        "a writer may not configure"
    );
    assert_eq!(
        as_user(bob.pair(), "GET", "").await,
        200,
        "a reader reads the configuration"
    );
    assert_eq!(
        as_user(ann.pair(), "PUT", CONFIGURATION).await,
        200,
        "the bucket's creator configures it"
    );
}
