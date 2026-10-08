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
    for size in [0_usize, 1, 64 * 1024, 128 * 1024, 6 << 20] {
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

/// The `legacy` bucket's incarnation, as a value to plant a record with.
async fn incarnation(planter: &MetaPool) -> Value {
    let answers = planter
        .run(
            "SELECT incarnation FROM ONLY buckets:$b;",
            vec![("b".to_owned(), Value::String("legacy".to_owned()))],
        )
        .await
        .expect("bucket read");
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => match records.into_iter().next() {
            Some((_, Value::Object(fields))) => {
                fields.get("incarnation").cloned().expect("incarnation")
            }
            other => panic!("no bucket: {other:?}"),
        },
        other => panic!("no bucket: {other:?}"),
    }
}

/// Whether the `objects` table still declares the `inline` bytes column.
async fn declares_inline(planter: &MetaPool) -> bool {
    let answers = planter
        .run("INFO FOR TABLE objects;", vec![])
        .await
        .expect("table info");
    let Some(Answer::Value {
        value: Value::Object(info),
        ..
    }) = answers.into_iter().next()
    else {
        panic!("no table info");
    };
    let Some(Value::Array(fields)) = info.get("fields") else {
        panic!("no fields: {info:?}");
    };
    fields.iter().any(|field| {
        matches!(field, Value::Object(f) if f.get("name") == Some(&Value::String("inline".to_owned())))
    })
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_store_holding_object_bytes_is_converted_at_start_and_then_refuses_them() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/legacy", vec![], b"").await.status,
        200
    );
    // The shape a build before ADR-0008 left: the column, and a small object's bytes in its record.
    planter
        .run("DEFINE FIELD inline ON objects TYPE bytes;", vec![])
        .await
        .expect("the old column");
    planter
        .run(
            "CREATE objects:'legacy/old' = { bucket_name: 'legacy', key: 'old', incarnation: $i, size: 3, \
             etag: '\"900150983cd24fb0d6963f7d28e17f72\"', modified: time::now(), headers: {}, metadata: {}, \
             checksums: {}, inline: $bytes };",
            vec![
                ("i".to_owned(), incarnation(&planter).await),
                ("bytes".to_owned(), Value::Bytes(b"abc".to_vec())),
            ],
        )
        .await
        .expect("plant");
    let refused = call(&state, "GET", "/legacy/old", vec![], b"").await;
    assert_eq!(
        refused.status, 500,
        "a record holding bytes is never served: {}",
        refused.body
    );

    let objects = state.storage().objects();
    assert_eq!(objects.convert_inline().await.expect("converted"), 1);
    let fields = record(&planter, "legacy", "old").await;
    assert!(
        !fields.contains_key("inline"),
        "{:?}",
        fields.keys().collect::<Vec<_>>()
    );
    assert!(matches!(fields.get("data"), Some(Value::Uuid(_))));
    let read = call(&state, "GET", "/legacy/old", vec![], b"").await;
    assert_eq!(
        (read.status, read.bytes.as_slice()),
        (200, &b"abc"[..]),
        "{}",
        read.body
    );
    assert!(
        !declares_inline(&planter).await,
        "the column is dropped once no record holds bytes"
    );
    assert_eq!(
        objects.convert_inline().await.expect("again"),
        0,
        "idempotent"
    );
    // From here the store itself refuses bytes in an object record.
    assert!(
        planter
            .run("UPDATE objects:'legacy/old' SET inline = 0x61;", vec![])
            .await
            .is_err(),
        "the strict table refuses the column it no longer declares"
    );
}
