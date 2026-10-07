//! UploadPartCopy against a real node: a part made of a source's bytes (a range or the whole), ETag'd by the MD5 of
//! those bytes, completing with uploaded parts into exactly their concatenation; every refusal by name, an unknown
//! upload before the source is even looked at.

use crate::large::body;
use crate::multipart::{create, etag, md5_hex, part, text, texts};
use crate::{IGNORED, call, call_with, fresh};

const MIB: usize = 1024 * 1024;

async fn copy_part(
    state: &tessari_s3_api::ApiState,
    path: &str,
    upload: &str,
    number: &str,
    headers: Vec<(&str, &str)>,
) -> crate::Seen {
    call_with(
        state,
        "PUT",
        path,
        vec![("partNumber", Some(number)), ("uploadId", Some(upload))],
        headers,
        b"",
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn copied_parts_complete_with_uploaded_ones_into_their_exact_concatenation() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cpp", vec![], b"").await.status, 200);
    let large = body(6 * MIB + 100, 5);
    assert_eq!(
        call(&state, "PUT", "/cpp/large", vec![], &large)
            .await
            .status,
        200
    );
    assert_eq!(
        call(&state, "PUT", "/cpp/small", vec![], b"small-tail")
            .await
            .status,
        200
    );
    let upload = create(&state, "/cpp/joined").await;
    let first = body(5 * MIB, 9);
    let uploaded = part(&state, "/cpp/joined", &upload, "1", &first).await;
    assert_eq!(uploaded.status, 200, "{}", uploaded.body);
    let range = format!("bytes=0-{}", 5 * MIB - 1);
    let ranged = copy_part(
        &state,
        "/cpp/joined",
        &upload,
        "2",
        vec![
            ("x-amz-copy-source", "cpp/large"),
            ("x-amz-copy-source-range", &range),
        ],
    )
    .await;
    assert_eq!(ranged.status, 200, "{}", ranged.body);
    assert!(ranged.body.contains("<CopyPartResult"), "{}", ranged.body);
    assert_eq!(
        text(&ranged.body, "ETag"),
        Some(format!("\"{}\"", md5_hex(&large[..5 * MIB]))),
        "the MD5 of the copied range"
    );
    let whole = copy_part(
        &state,
        "/cpp/joined",
        &upload,
        "3",
        vec![("x-amz-copy-source", "/cpp/small")],
    )
    .await;
    assert_eq!(whole.status, 200, "{}", whole.body);
    let whole_etag = text(&whole.body, "ETag").expect("an ETag");
    assert_eq!(whole_etag, format!("\"{}\"", md5_hex(b"small-tail")));
    let listed = call(
        &state,
        "GET",
        "/cpp/joined",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(
        texts(&listed.body, "Part", "Size"),
        [
            (5 * MIB).to_string(),
            (5 * MIB).to_string(),
            "10".to_owned()
        ]
    );
    let document = format!(
        "<CompleteMultipartUpload>\
         <Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part>\
         <Part><PartNumber>2</PartNumber><ETag>{}</ETag></Part>\
         <Part><PartNumber>3</PartNumber><ETag>{whole_etag}</ETag></Part></CompleteMultipartUpload>",
        etag(&uploaded),
        text(&ranged.body, "ETag").expect("an ETag"),
    );
    let done = call(
        &state,
        "POST",
        "/cpp/joined",
        vec![("uploadId", Some(upload.as_str()))],
        document.as_bytes(),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let mut expected = first;
    expected.extend_from_slice(&large[..5 * MIB]);
    expected.extend_from_slice(b"small-tail");
    let read = call(&state, "GET", "/cpp/joined", vec![], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(read.bytes == expected, "the concatenation of the parts");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_part_copy_is_refused_by_name() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cpq", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/cpq/src", vec![], b"0123456789")
            .await
            .status,
        200
    );
    let upload = create(&state, "/cpq/dst").await;
    let unknown = "0".repeat(32);
    let no_upload = copy_part(
        &state,
        "/cpq/dst",
        &unknown,
        "1",
        vec![("x-amz-copy-source", "cpq/missing")],
    )
    .await;
    assert_eq!(
        no_upload.code.as_deref(),
        Some("NoSuchUpload"),
        "the upload is checked before the source: {}",
        no_upload.body
    );
    for (headers, code) in [
        (vec![("x-amz-copy-source", "cpq/missing")], "NoSuchKey"),
        (
            vec![
                ("x-amz-copy-source", "cpq/src"),
                ("x-amz-copy-source-range", "bytes=5-"),
            ],
            "InvalidArgument",
        ),
        (
            vec![
                ("x-amz-copy-source", "cpq/src"),
                ("x-amz-copy-source-range", "bytes=0-10"),
            ],
            "InvalidRequest",
        ),
        (
            vec![
                ("x-amz-copy-source", "cpq/src"),
                ("x-amz-copy-source-if-match", "\"0000\""),
            ],
            "PreconditionFailed",
        ),
    ] {
        let refused = copy_part(&state, "/cpq/dst", &upload, "1", headers.clone()).await;
        assert_eq!(
            refused.code.as_deref(),
            Some(code),
            "{headers:?}: {}",
            refused.body
        );
    }
    let listed = call(
        &state,
        "GET",
        "/cpq/dst",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(
        texts(&listed.body, "Part", "PartNumber").len(),
        0,
        "no part was written"
    );
}
