//! CopyObject against a real node: the bytes, the metadata (copied or replaced) and a checksum recomputed over the
//! copy; a data-file or multipart source becomes a single-part object of its own that outlives the source; every
//! refusal by name, a too-large source before any byte is read.

use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Hashes};
use tessari_s3_infrastructure::tessaridb::{Answer, Value};

use crate::large::body;
use crate::multipart::{create, etag, md5_hex, part, text};
use crate::{IGNORED, call, call_with, fresh};

fn crc64(bytes: &[u8]) -> String {
    let mut hashes = Hashes::new();
    hashes.update(bytes);
    hashes
        .finish()
        .value(ChecksumAlgorithm::Crc64Nvme)
        .to_owned()
}

fn header<'a>(seen: &'a crate::Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_copy_keeps_or_replaces_the_metadata_and_carries_its_own_etag_and_checksum() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cpy", vec![], b"").await.status, 200);
    let put = call_with(
        &state,
        "PUT",
        "/cpy/src",
        vec![],
        vec![
            ("content-type", "text/plain"),
            ("x-amz-meta-colour", "blue"),
        ],
        b"small source",
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    let copied = call_with(
        &state,
        "PUT",
        "/cpy/dst",
        vec![],
        vec![
            ("x-amz-copy-source", "/cpy/src"),
            ("content-type", "ignored/without-replace"),
        ],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    let quoted = format!("\"{}\"", md5_hex(b"small source"));
    assert_eq!(
        text(&copied.body, "ETag"),
        Some(quoted.clone()),
        "{}",
        copied.body
    );
    assert_eq!(
        text(&copied.body, "ChecksumCRC64NVME"),
        Some(crc64(b"small source"))
    );
    let read = call(&state, "GET", "/cpy/dst", vec![], b"").await;
    assert_eq!(read.bytes, b"small source");
    assert_eq!(
        header(&read, "content-type"),
        Some("text/plain"),
        "COPY keeps it"
    );
    assert_eq!(header(&read, "x-amz-meta-colour"), Some("blue"));
    assert_eq!(header(&read, "etag"), Some(quoted.as_str()));
    let replaced = call_with(
        &state,
        "PUT",
        "/cpy/dst2",
        vec![],
        vec![
            ("x-amz-copy-source", "cpy/src"),
            ("x-amz-metadata-directive", "REPLACE"),
            ("content-type", "application/json"),
            ("x-amz-meta-shape", "round"),
        ],
        b"",
    )
    .await;
    assert_eq!(replaced.status, 200, "{}", replaced.body);
    let read = call(&state, "GET", "/cpy/dst2", vec![], b"").await;
    assert_eq!(header(&read, "content-type"), Some("application/json"));
    assert_eq!(header(&read, "x-amz-meta-shape"), Some("round"));
    assert_eq!(
        header(&read, "x-amz-meta-colour"),
        None,
        "REPLACE drops the source's"
    );
    // Onto itself only when something changes.
    let onto_itself = call_with(
        &state,
        "PUT",
        "/cpy/src",
        vec![],
        vec![("x-amz-copy-source", "cpy/src")],
        b"",
    )
    .await;
    assert_eq!(
        onto_itself.code.as_deref(),
        Some("InvalidRequest"),
        "{}",
        onto_itself.body
    );
    let renewed = call_with(
        &state,
        "PUT",
        "/cpy/src",
        vec![],
        vec![
            ("x-amz-copy-source", "cpy/src"),
            ("x-amz-metadata-directive", "REPLACE"),
            ("x-amz-meta-colour", "red"),
        ],
        b"",
    )
    .await;
    assert_eq!(renewed.status, 200, "{}", renewed.body);
    let read = call(&state, "GET", "/cpy/src", vec![], b"").await;
    assert_eq!(read.bytes, b"small source");
    assert_eq!(header(&read, "x-amz-meta-colour"), Some("red"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_data_or_multipart_source_becomes_a_single_part_object_of_its_own() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cpl", vec![], b"").await.status, 200);
    let large = body(300 * 1024, 3);
    assert_eq!(
        call(&state, "PUT", "/cpl/large", vec![], &large)
            .await
            .status,
        200
    );
    let copied = call_with(
        &state,
        "PUT",
        "/cpl/large-copy",
        vec![],
        vec![("x-amz-copy-source", "cpl/large")],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    assert_eq!(
        text(&copied.body, "ETag"),
        Some(format!("\"{}\"", md5_hex(&large)))
    );
    // The copy has its own file: overwriting and deleting the source leaves it whole.
    let data_of = |key: &'static str| {
        let planter = planter.clone();
        async move {
            let answers = planter
                .run(
                    "SELECT data FROM ONLY objects:$id;",
                    vec![("id".to_owned(), Value::String(format!("cpl/{key}")))],
                )
                .await
                .expect("object read");
            match answers.into_iter().next() {
                Some(Answer::Records { records, .. }) => records.into_iter().next().map(|(_, v)| v),
                Some(Answer::Value { value, .. }) => Some(value),
                _ => None,
            }
        }
    };
    let (source_data, copy_data) = (data_of("large").await, data_of("large-copy").await);
    assert!(source_data.is_some(), "the source is a data object");
    assert_ne!(
        source_data, copy_data,
        "the copy does not share the source's file"
    );
    assert_eq!(
        call(&state, "PUT", "/cpl/large", vec![], b"gone")
            .await
            .status,
        200
    );
    assert_eq!(
        call(&state, "DELETE", "/cpl/large", vec![], b"")
            .await
            .status,
        204
    );
    let read = call(&state, "GET", "/cpl/large-copy", vec![], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(read.bytes == large, "the copy's bytes");
    // A multipart source is copied as data: one part, a plain MD5 ETag, a full-object checksum.
    let first = body(5 * 1024 * 1024, 4);
    let upload = create(&state, "/cpl/parts").await;
    let one = part(&state, "/cpl/parts", &upload, "1", &first).await;
    let two = part(&state, "/cpl/parts", &upload, "2", b"tail of it").await;
    let document = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part>\
         <Part><PartNumber>2</PartNumber><ETag>{}</ETag></Part></CompleteMultipartUpload>",
        etag(&one),
        etag(&two)
    );
    let done = call(
        &state,
        "POST",
        "/cpl/parts",
        vec![("uploadId", Some(upload.as_str()))],
        document.as_bytes(),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let mut whole = first.clone();
    whole.extend_from_slice(b"tail of it");
    let copied = call_with(
        &state,
        "PUT",
        "/cpl/parts-copy",
        vec![],
        vec![("x-amz-copy-source", "cpl/parts")],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    assert_eq!(
        text(&copied.body, "ETag"),
        Some(format!("\"{}\"", md5_hex(&whole)))
    );
    assert_eq!(text(&copied.body, "ChecksumCRC64NVME"), Some(crc64(&whole)));
    assert_eq!(
        text(&copied.body, "ChecksumType").as_deref(),
        Some("FULL_OBJECT")
    );
    assert_eq!(
        call(&state, "DELETE", "/cpl/parts", vec![], b"")
            .await
            .status,
        204
    );
    let read = call(
        &state,
        "GET",
        "/cpl/parts-copy",
        vec![("partNumber", Some("1"))],
        b"",
    )
    .await;
    assert_eq!(read.status, 200, "a single-part object is its own part 1");
    assert!(read.bytes == whole, "the copy's bytes");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_copy_is_refused_by_name_before_it_writes() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cpr", vec![], b"").await.status, 200);
    let put = call(&state, "PUT", "/cpr/src", vec![], b"source").await;
    let source_etag = etag(&put);
    let copy = |headers: Vec<(&'static str, String)>| {
        let state = state.clone();
        async move {
            let headers: Vec<(&str, &str)> =
                headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            call_with(&state, "PUT", "/cpr/dst", vec![], headers, b"").await
        }
    };
    let source = |value: &str| ("x-amz-copy-source", value.to_owned());
    for (headers, code) in [
        (vec![source("cpr/missing")], "NoSuchKey"),
        (vec![source("cpr-nowhere/src")], "NoSuchBucket"),
        (vec![source("cpr/src?versionId=1")], "NotImplemented"),
        (vec![source("cpr")], "InvalidArgument"),
        (
            vec![
                source("cpr/src"),
                ("x-amz-copy-source-if-match", "\"0000\"".to_owned()),
            ],
            "PreconditionFailed",
        ),
        (
            vec![
                source("cpr/src"),
                ("x-amz-copy-source-if-none-match", source_etag.clone()),
            ],
            "PreconditionFailed",
        ),
        (
            vec![
                source("cpr/src"),
                (
                    "x-amz-copy-source-server-side-encryption-customer-algorithm",
                    "AES256".to_owned(),
                ),
            ],
            "NotImplemented",
        ),
        (
            vec![
                source("cpr/src"),
                ("x-amz-metadata-directive", "MOVE".to_owned()),
            ],
            "InvalidArgument",
        ),
    ] {
        let refused = copy(headers.clone()).await;
        assert_eq!(
            refused.code.as_deref(),
            Some(code),
            "{headers:?}: {}",
            refused.body
        );
    }
    let missing = call(&state, "GET", "/cpr/dst", vec![], b"").await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchKey"),
        "nothing was written"
    );
    // A condition that holds copies; the destination's own If-None-Match is a compare-and-set.
    let held = copy(vec![
        source("cpr/src"),
        ("x-amz-copy-source-if-match", source_etag.clone()),
    ])
    .await;
    assert_eq!(held.status, 200, "{}", held.body);
    let exists = copy(vec![source("cpr/src"), ("if-none-match", "*".to_owned())]).await;
    assert_eq!(
        exists.code.as_deref(),
        Some("PreconditionFailed"),
        "{}",
        exists.body
    );
    // A source past 5 GiB is refused before a byte of it is read (its data file does not even exist).
    let incarnation = incarnation_value(&planter).await;
    planter
        .run(
            "CREATE objects:'cpr/huge' = { bucket_name: 'cpr', key: 'huge', incarnation: $i, size: 6442450944, \
             etag: '\"e\"', modified: time::now(), headers: {}, metadata: {}, checksums: {}, data: rand::uuid() };",
            vec![("i".to_owned(), incarnation)],
        )
        .await
        .expect("plant");
    let huge = copy(vec![source("cpr/huge")]).await;
    assert_eq!(
        huge.code.as_deref(),
        Some("InvalidRequest"),
        "{}",
        huge.body
    );
    // A source whose bytes no longer match its stored whole-object checksum is not copied.
    planter
        .run(
            "CREATE objects:'cpr/bent' = { bucket_name: 'cpr', key: 'bent', incarnation: $i, size: 3, \
             etag: '\"e\"', modified: time::now(), headers: {}, metadata: {}, \
             checksums: { CRC64NVME: 'AAAAAAAAAAA=' }, inline: $bytes };",
            vec![
                ("i".to_owned(), incarnation_value(&planter).await),
                ("bytes".to_owned(), Value::Bytes(b"abc".to_vec())),
            ],
        )
        .await
        .expect("plant");
    let bent = call_with(
        &state,
        "PUT",
        "/cpr/from-bent",
        vec![],
        vec![("x-amz-copy-source", "cpr/bent")],
        b"",
    )
    .await;
    assert_eq!(bent.code.as_deref(), Some("InternalError"), "{}", bent.body);
    let none = call(&state, "GET", "/cpr/from-bent", vec![], b"").await;
    assert_eq!(
        none.code.as_deref(),
        Some("NoSuchKey"),
        "nothing was written"
    );
}

/// The `cpr` bucket's incarnation, as a value to plant records with.
async fn incarnation_value(planter: &tessari_s3_infrastructure::tessaridb::MetaPool) -> Value {
    let answer = planter
        .run(
            "SELECT incarnation FROM ONLY buckets:$b;",
            vec![("b".to_owned(), Value::String("cpr".to_owned()))],
        )
        .await
        .expect("bucket read")
        .into_iter()
        .next();
    let fields = match answer {
        Some(Answer::Records { records, .. }) => match records.into_iter().next() {
            Some((_, Value::Object(fields))) => fields,
            other => panic!("no bucket: {other:?}"),
        },
        Some(Answer::Value {
            value: Value::Object(fields),
            ..
        }) => fields,
        other => panic!("no bucket: {other:?}"),
    };
    fields.get("incarnation").cloned().expect("incarnation")
}
