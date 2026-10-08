//! No object byte is held in the metadata store (ADR-0008): every object, however small, is a data file on the
//! drive, and its record carries the size, ETag, checksums and the data reference — never the bytes.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{IGNORED, call, fresh};

/// The stored record of `bucket/key`, as the metadata store holds it.
async fn record(planter: &MetaPool, bucket: &str, key: &str) -> BTreeMap<String, Value> {
    let answers = planter
        .run(
            "SELECT * FROM ONLY objects:$id;",
            vec![("id".to_owned(), Value::String(format!("{bucket}/{key}")))],
        )
        .await
        .expect("the record reads");
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => match records.into_iter().next() {
            Some((_, Value::Object(fields))) => fields,
            other => panic!("not a record: {other:?}"),
        },
        other => panic!("not a record: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_small_object_is_a_data_file_and_its_record_holds_no_bytes() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/nobytes", vec![], b"").await.status,
        200
    );
    for size in [0_usize, 1, 64 * 1024, 128 * 1024] {
        let body = vec![b'x'; size];
        let key = format!("k{size}");
        let put = call(&state, "PUT", &format!("/nobytes/{key}"), vec![], &body).await;
        assert_eq!(put.status, 200, "{size} B: {}", put.body);
        let fields = record(&planter, "nobytes", &key).await;
        assert!(
            !fields.contains_key("inline"),
            "{size} B: the record carries object bytes: {:?}",
            fields.keys().collect::<Vec<_>>()
        );
        assert!(
            matches!(fields.get("data"), Some(Value::Uuid(_))),
            "{size} B: the record names no data file: {:?}",
            fields.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            fields.get("size"),
            Some(&Value::from(i64::try_from(size).expect("size fits"))),
            "{size} B"
        );
        for field in ["etag", "checksums", "headers", "metadata", "modified"] {
            assert!(fields.contains_key(field), "{size} B: no {field}");
        }
        let read = call(&state, "GET", &format!("/nobytes/{key}"), vec![], b"").await;
        assert_eq!(read.status, 200, "{size} B: {}", read.body);
        assert_eq!(read.bytes, body, "{size} B: the bytes read back");
    }
}
