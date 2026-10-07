//! ListMultipartUploads against a real node: open uploads by key in byte order and, within a key, by initiation;
//! paged by key-marker and upload-id-marker without a repeat or a gap; rolled up at a delimiter; an upload that ends
//! leaves the listing; another bucket's uploads and an earlier incarnation's never appear.

use crate::multipart::{create, etag, part, text, texts};
use crate::{IGNORED, call, fresh};

/// `GET /{bucket}?uploads` with `query`; the answer must be a listing.
async fn uploads(state: &tessari_s3_api::ApiState, bucket: &str, query: &[(&str, &str)]) -> String {
    let mut pairs = vec![("uploads", None)];
    pairs.extend(query.iter().map(|(k, v)| (*k, Some(*v))));
    let listed = call(state, "GET", bucket, pairs, b"").await;
    assert_eq!(listed.status, 200, "{}", listed.body);
    assert!(
        listed.body.contains("<ListMultipartUploadsResult"),
        "{}",
        listed.body
    );
    listed.body
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn open_uploads_list_by_key_then_initiation_and_page_by_markers() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    for bucket in ["/mpu", "/mpu-n"] {
        assert_eq!(call(&state, "PUT", bucket, vec![], b"").await.status, 200);
    }
    let b = create(&state, "/mpu/b").await;
    let a1 = create(&state, "/mpu/a").await;
    let a2 = create(&state, "/mpu/a").await;
    let ax = create(&state, "/mpu/a/x").await;
    let c = create(&state, "/mpu/c").await;
    let _neighbour = create(&state, "/mpu-n/a").await;
    let all = uploads(&state, "/mpu", &[]).await;
    assert_eq!(texts(&all, "Upload", "Key"), ["a", "a", "a/x", "b", "c"]);
    let order = [a1.clone(), a2.clone(), ax, b.clone(), c.clone()];
    assert_eq!(
        texts(&all, "Upload", "UploadId"),
        order,
        "one key's uploads by initiation"
    );
    assert_eq!(text(&all, "IsTruncated").as_deref(), Some("false"));
    assert_eq!(texts(&all, "Upload", "Initiated").len(), 5);
    // One upload per page, resumed from the markers each page names.
    let mut seen = Vec::new();
    let mut markers: Vec<(String, String)> = Vec::new();
    loop {
        let mut query = vec![("max-uploads", "1")];
        if let Some((key, id)) = markers.last() {
            query.push(("key-marker", key.as_str()));
            query.push(("upload-id-marker", id.as_str()));
        }
        let page = uploads(&state, "/mpu", &query).await;
        seen.extend(texts(&page, "Upload", "UploadId"));
        if text(&page, "IsTruncated").as_deref() != Some("true") {
            break;
        }
        let next = (
            text(&page, "NextKeyMarker").expect("a next key marker"),
            text(&page, "NextUploadIdMarker").expect("a next upload marker"),
        );
        assert!(seen.len() <= order.len(), "the pages do not end: {seen:?}");
        markers.push(next);
    }
    assert_eq!(seen, order, "paging neither repeats nor skips");
    // A key-marker alone starts past every upload of that key.
    let past_a = uploads(&state, "/mpu", &[("key-marker", "a")]).await;
    assert_eq!(texts(&past_a, "Upload", "Key"), ["a/x", "b", "c"]);
    // An upload that ends leaves the listing.
    let aborted = call(
        &state,
        "DELETE",
        "/mpu/b",
        vec![("uploadId", Some(b.as_str()))],
        b"",
    )
    .await;
    assert_eq!(aborted.status, 204, "{}", aborted.body);
    let after = uploads(&state, "/mpu", &[]).await;
    assert_eq!(texts(&after, "Upload", "Key"), ["a", "a", "a/x", "c"]);
    // So does one that completes.
    let only = part(&state, "/mpu/c", &c, "1", b"last").await;
    assert_eq!(only.status, 200, "{}", only.body);
    let document = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part></CompleteMultipartUpload>",
        etag(&only)
    );
    let completed = call(
        &state,
        "POST",
        "/mpu/c",
        vec![("uploadId", Some(c.as_str()))],
        document.as_bytes(),
    )
    .await;
    assert_eq!(completed.status, 200, "{}", completed.body);
    let done = uploads(&state, "/mpu", &[]).await;
    assert_eq!(texts(&done, "Upload", "Key"), ["a", "a", "a/x"]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn uploads_roll_up_once_and_a_vanished_marker_repeats_rather_than_skips() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/mpr", vec![], b"").await.status, 200);
    for key in ["/mpr/d/1", "/mpr/d/1", "/mpr/d/2", "/mpr/e"] {
        create(&state, key).await;
    }
    let rolled = uploads(&state, "/mpr", &[("delimiter", "/")]).await;
    assert_eq!(texts(&rolled, "CommonPrefixes", "Prefix"), ["d/"]);
    assert_eq!(texts(&rolled, "Upload", "Key"), ["e"]);
    let first = uploads(&state, "/mpr", &[("delimiter", "/"), ("max-uploads", "1")]).await;
    assert_eq!(
        texts(&first, "CommonPrefixes", "Prefix"),
        ["d/"],
        "a prefix counts once"
    );
    assert_eq!(text(&first, "IsTruncated").as_deref(), Some("true"));
    assert_eq!(text(&first, "NextKeyMarker").as_deref(), Some("d/"));
    let second = uploads(
        &state,
        "/mpr",
        &[
            ("delimiter", "/"),
            ("max-uploads", "1"),
            ("key-marker", "d/"),
        ],
    )
    .await;
    assert_eq!(texts(&second, "CommonPrefixes", "Prefix").len(), 0);
    assert_eq!(texts(&second, "Upload", "Key"), ["e"]);
    // The marker's upload ended between pages: its key is listed again rather than skipped.
    let g1 = create(&state, "/mpr/g").await;
    let g2 = create(&state, "/mpr/g").await;
    let gone = call(
        &state,
        "DELETE",
        "/mpr/g",
        vec![("uploadId", Some(g1.as_str()))],
        b"",
    )
    .await;
    assert_eq!(gone.status, 204, "{}", gone.body);
    let resumed = uploads(
        &state,
        "/mpr",
        &[("key-marker", "g"), ("upload-id-marker", g1.as_str())],
    )
    .await;
    assert_eq!(texts(&resumed, "Upload", "UploadId"), [g2]);
    let prefixed = uploads(&state, "/mpr", &[("prefix", "d/")]).await;
    assert_eq!(texts(&prefixed, "Upload", "Key"), ["d/1", "d/1", "d/2"]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_earlier_incarnations_uploads_are_not_listed_and_a_missing_bucket_is_refused() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let missing = call(&state, "GET", "/mpx", vec![("uploads", None)], b"").await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchBucket"),
        "{}",
        missing.body
    );
    assert_eq!(call(&state, "PUT", "/mpx", vec![], b"").await.status, 200);
    create(&state, "/mpx/old").await;
    assert_eq!(
        call(&state, "DELETE", "/mpx", vec![], b"").await.status,
        204
    );
    assert_eq!(call(&state, "PUT", "/mpx", vec![], b"").await.status, 200);
    let fresh_bucket = uploads(&state, "/mpx", &[]).await;
    assert_eq!(
        texts(&fresh_bucket, "Upload", "Key").len(),
        0,
        "{fresh_bucket}"
    );
}
