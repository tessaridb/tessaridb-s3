//! Media processing against a real node (G004): a bucket's media rules, and the changes a media-only bucket records so
//! its media worker can find them.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};

use crate::notifications::recorded;
use crate::{IGNORED, call, fresh};

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
