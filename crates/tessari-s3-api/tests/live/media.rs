//! Media processing against a real node (G004): a bucket's media rules, and the changes a media-only bucket records so
//! its media worker can find them.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use crate::notifications::recorded;
use crate::{IGNORED, call, fresh};

/// How many messages the media group holds in flight, as the topic reports it.
async fn in_flight(planter: &MetaPool) -> i64 {
    let answers = planter
        .run("INFO FOR TOPIC object_events;", Vec::new())
        .await
        .expect("topic described");
    let Some(Answer::Value { value, .. }) = answers.into_iter().next() else {
        panic!("INFO FOR TOPIC answered no value");
    };
    let field = |value: Option<Value>, name: &str| match value {
        Some(Value::Object(mut fields)) => fields.remove(name),
        _ => None,
    };
    match field(field(field(Some(value), "groups"), "media"), "in_flight") {
        Some(Value::Number(Number::Integer(count))) => count,
        other => panic!("no media group in the topic's description: {other:?}"),
    }
}

/// Gives `bucket` media rules as the console API stores them.
async fn with_media(planter: &MetaPool, bucket: &str) {
    planter
        .run(
            "UPDATE buckets:$name SET media = [{ id: 'photos', content_types: ['image/*'], prefix: '', \
             outputs: ['thumb'] }];",
            vec![("name".to_owned(), Value::String(bucket.to_owned()))],
        )
        .await
        .expect("media rules set");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_with_media_rules_records_its_changes_without_a_notification_configuration() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for name in ["pictures", "plain"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    with_media(&planter, "pictures").await;
    call(&state, "PUT", "/pictures/a.jpg", vec![], b"jpeg").await;
    call(&state, "PUT", "/plain/b.jpg", vec![], b"jpeg").await;
    assert_eq!(
        recorded(&planter, "a.jpg").await,
        vec!["s3:ObjectCreated:Put".to_owned()]
    );
    assert!(
        recorded(&planter, "b.jpg").await.is_empty(),
        "a bucket with neither rules nor a configuration records nothing"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_media_group_hands_out_only_objects_created_where_a_rule_takes_them() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for name in ["pictures", "plain"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    with_media(&planter, "pictures").await;
    call(&state, "PUT", "/pictures/a.jpg", vec![], b"jpeg").await;
    call(
        &state,
        "PUT",
        "/pictures/a.jpg.renditions/thumb.jpg",
        vec![],
        b"jpeg",
    )
    .await;
    call(&state, "DELETE", "/pictures/a.jpg", vec![], b"").await;
    call(&state, "PUT", "/plain/b.jpg", vec![], b"jpeg").await;
    let media = state.storage().media();
    media.define().await.expect("group defined");
    let jobs = media.next(16).await.expect("jobs read");
    let keys: Vec<(&str, &str)> = jobs
        .iter()
        .map(|job| (job.event.bucket.as_str(), job.event.key.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![("pictures", "a.jpg")],
        "a removal, a rendition and a bucket without rules make no job"
    );
    assert_eq!(
        in_flight(&planter).await,
        1,
        "what no rule takes is settled at once; only the job is held"
    );
    for job in &jobs {
        media.done(job).await.expect("settled");
    }
    assert_eq!(in_flight(&planter).await, 0, "a done job is settled");
    assert!(
        media.next(16).await.expect("jobs read").is_empty(),
        "what was settled is not handed out again"
    );
}
