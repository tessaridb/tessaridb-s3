//! PutObject, GetObject, HeadObject and DeleteObject against a real node.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Seen, call, call_with, fresh};

const IGNORED: &str = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD";

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|v| v.to_str().ok())
}

async fn bucket(state: &tessari_s3_api::ApiState, name: &str) {
    assert_eq!(
        call(state, "PUT", &format!("/{name}"), vec![], b"")
            .await
            .status,
        200
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_is_written_read_described_and_deleted() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    bucket(&state, "docs").await;
    let put = call_with(
        &state,
        "PUT",
        "/docs/a/b.txt",
        vec![],
        vec![("content-type", "text/plain"), ("x-amz-meta-Color", "blue")],
        b"hello",
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(
        header(&put, "etag"),
        Some("\"5d41402abc4b2a76b9719d911017c592\""),
        "the MD5 of the body"
    );
    let got = call(&state, "GET", "/docs/a/b.txt", vec![], b"").await;
    assert_eq!((got.status, got.body.as_str()), (200, "hello"));
    assert_eq!(header(&got, "content-type"), Some("text/plain"));
    assert_eq!(header(&got, "x-amz-meta-color"), Some("blue"));
    assert_eq!(header(&got, "etag"), header(&put, "etag"));
    assert!(header(&got, "last-modified").is_some_and(|d| d.ends_with(" GMT")));
    let head = call(&state, "HEAD", "/docs/a/b.txt", vec![], b"").await;
    assert_eq!(
        (
            head.status,
            head.body.as_str(),
            header(&head, "content-length")
        ),
        (200, "", Some("5"))
    );
    assert_eq!(
        call(&state, "DELETE", "/docs/a/b.txt", vec![], b"")
            .await
            .status,
        204
    );
    let gone = call(&state, "GET", "/docs/a/b.txt", vec![], b"").await;
    assert_eq!(
        (gone.status, gone.code.as_deref()),
        (404, Some("NoSuchKey"))
    );
    assert_eq!(
        call(&state, "DELETE", "/docs/a/b.txt", vec![], b"")
            .await
            .status,
        204,
        "deleting a missing key succeeds"
    );
    let nowhere = call(&state, "PUT", "/absent-bucket/k", vec![], b"x").await;
    assert_eq!(
        (nowhere.status, nowhere.code.as_deref()),
        (404, Some("NoSuchBucket"))
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn checksums_are_validated_stored_and_returned() {
    let (state, _) = fresh().await;
    bucket(&state, "sums").await;
    let good = call_with(
        &state,
        "PUT",
        "/sums/k",
        vec![],
        vec![("x-amz-checksum-crc32", "NhCmhg==")],
        b"hello",
    )
    .await;
    assert_eq!(
        (good.status, header(&good, "x-amz-checksum-crc32")),
        (200, Some("NhCmhg==")),
        "{}",
        good.body
    );
    let bad = call_with(
        &state,
        "PUT",
        "/sums/bad",
        vec![],
        vec![("x-amz-checksum-crc32", "AAAAAA==")],
        b"hello",
    )
    .await;
    assert_eq!((bad.status, bad.code.as_deref()), (400, Some("BadDigest")));
    assert_eq!(
        call(&state, "GET", "/sums/bad", vec![], b"").await.status,
        404,
        "a refused body stores nothing"
    );
    let md5 = call_with(
        &state,
        "PUT",
        "/sums/md5",
        vec![],
        vec![("content-md5", "AAAAAAAAAAAAAAAAAAAAAA==")],
        b"hello",
    )
    .await;
    assert_eq!((md5.status, md5.code.as_deref()), (400, Some("BadDigest")));
    let read = call_with(
        &state,
        "GET",
        "/sums/k",
        vec![],
        vec![("x-amz-checksum-mode", "ENABLED")],
        b"",
    )
    .await;
    assert_eq!(header(&read, "x-amz-checksum-crc32"), Some("NhCmhg=="));
    assert_eq!(
        call(&state, "PUT", "/sums/default", vec![], b"123456789")
            .await
            .status,
        200
    );
    let default = call_with(
        &state,
        "GET",
        "/sums/default",
        vec![],
        vec![("x-amz-checksum-mode", "ENABLED")],
        b"",
    )
    .await;
    assert_eq!(
        header(&default, "x-amz-checksum-crc64nvme"),
        Some("rosUhgp5mIg="),
        "CRC64NVME is stored by default"
    );
    let plain = call(&state, "GET", "/sums/default", vec![], b"").await;
    assert_eq!(
        header(&plain, "x-amz-checksum-crc64nvme"),
        None,
        "only when the client asks"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn conditional_writes_are_one_compare_and_set() {
    let (state, _) = fresh().await;
    bucket(&state, "cas").await;
    let create = call_with(
        &state,
        "PUT",
        "/cas/k",
        vec![],
        vec![("if-none-match", "*")],
        b"one",
    )
    .await;
    assert_eq!(create.status, 200);
    let again = call_with(
        &state,
        "PUT",
        "/cas/k",
        vec![],
        vec![("if-none-match", "*")],
        b"two",
    )
    .await;
    assert_eq!(
        (again.status, again.code.as_deref()),
        (412, Some("PreconditionFailed"))
    );
    let etag = header(&create, "etag").expect("etag").to_owned();
    let swapped = call_with(
        &state,
        "PUT",
        "/cas/k",
        vec![],
        vec![("if-match", etag.as_str())],
        b"three",
    )
    .await;
    assert_eq!(swapped.status, 200);
    let stale = call_with(
        &state,
        "PUT",
        "/cas/k",
        vec![],
        vec![("if-match", etag.as_str())],
        b"four",
    )
    .await;
    assert_eq!(
        (stale.status, stale.code.as_deref()),
        (412, Some("PreconditionFailed"))
    );
    let missing = call_with(
        &state,
        "PUT",
        "/cas/none",
        vec![],
        vec![("if-match", etag.as_str())],
        b"x",
    )
    .await;
    assert_eq!(
        (missing.status, missing.code.as_deref()),
        (404, Some("NoSuchKey"))
    );
    assert_eq!(
        call(&state, "GET", "/cas/k", vec![], b"").await.body,
        "three"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn conditional_reads_and_ranges() {
    let (state, _) = fresh().await;
    bucket(&state, "reads").await;
    let put = call(&state, "PUT", "/reads/k", vec![], b"hello").await;
    let etag = header(&put, "etag").expect("etag").to_owned();
    let not_modified = call_with(
        &state,
        "GET",
        "/reads/k",
        vec![],
        vec![("if-none-match", etag.as_str())],
        b"",
    )
    .await;
    assert_eq!((not_modified.status, not_modified.body.as_str()), (304, ""));
    let failed = call_with(
        &state,
        "GET",
        "/reads/k",
        vec![],
        vec![("if-match", "\"nope\"")],
        b"",
    )
    .await;
    assert_eq!(
        (failed.status, failed.code.as_deref()),
        (412, Some("PreconditionFailed"))
    );
    let part = call_with(
        &state,
        "GET",
        "/reads/k",
        vec![],
        vec![("range", "bytes=1-3")],
        b"",
    )
    .await;
    assert_eq!(
        (
            part.status,
            part.body.as_str(),
            header(&part, "content-range")
        ),
        (206, "ell", Some("bytes 1-3/5"))
    );
    let beyond = call_with(
        &state,
        "GET",
        "/reads/k",
        vec![],
        vec![("range", "bytes=10-")],
        b"",
    )
    .await;
    assert_eq!(
        (beyond.status, beyond.code.as_deref()),
        (416, Some("InvalidRange"))
    );
    let overridden = call(
        &state,
        "GET",
        "/reads/k",
        vec![("response-content-type", Some("text/csv"))],
        b"",
    )
    .await;
    assert_eq!(header(&overridden, "content-type"), Some("text/csv"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_larger_than_the_inline_size_is_not_implemented_yet() {
    let (state, _) = fresh().await;
    bucket(&state, "big").await;
    let body = vec![b'x'; 128 * 1024 + 1];
    let refused = call(&state, "PUT", "/big/k", vec![], &body).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (501, Some("NotImplemented"))
    );
    assert_eq!(
        call(&state, "PUT", "/big/edge", vec![], &vec![b'x'; 128 * 1024])
            .await
            .status,
        200,
        "128 KiB fits"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_from_an_earlier_life_of_the_bucket_is_invisible_and_replaceable() {
    let (state, planter) = fresh().await;
    bucket(&state, "reborn").await;
    assert_eq!(
        call(&state, "PUT", "/reborn/k", vec![], b"old")
            .await
            .status,
        200
    );
    // The race this models: DeleteBucket and a PutObject commit disjoint records under snapshot isolation, so a
    // bucket can be removed while an object record of it remains. Remove the bucket record alone, then recreate.
    let name = vec![("name".to_owned(), Value::String("reborn".to_owned()))];
    planter
        .run("DELETE buckets:$name;", name)
        .await
        .expect("bucket record removed");
    bucket(&state, "reborn").await;
    let get = call(&state, "GET", "/reborn/k", vec![], b"").await;
    assert_eq!(
        (get.status, get.code.as_deref()),
        (404, Some("NoSuchKey")),
        "the old record is not this bucket's"
    );
    let create = call_with(
        &state,
        "PUT",
        "/reborn/k",
        vec![],
        vec![("if-none-match", "*")],
        b"new",
    )
    .await;
    assert_eq!(
        create.status, 200,
        "an absent key for this bucket, though a stale record sits there: {}",
        create.body
    );
    assert_eq!(
        call(&state, "GET", "/reborn/k", vec![], b"").await.body,
        "new"
    );
}
