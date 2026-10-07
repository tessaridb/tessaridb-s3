//! Multipart uploads against a real node: create, upload and replace parts, list them, abort. Parts are never
//! objects, an upload is bound to its key, and every byte an abort or a replacement drops is queued for reclamation.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::large::{body, queued};
use crate::{IGNORED, Seen, call, fresh};

fn text(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    body.split_once(&open)
        .and_then(|(_, rest)| rest.split_once(&close))
        .map(|(text, _)| text.replace("&quot;", "\"").replace("&amp;", "&"))
}

fn texts(body: &str, parent: &str, tag: &str) -> Vec<String> {
    body.split(&format!("<{parent}>"))
        .skip(1)
        .filter_map(|part| text(part, tag))
        .collect()
}

fn etag(seen: &Seen) -> String {
    seen.headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .to_owned()
}

async fn create(state: &tessari_s3_api::ApiState, path: &str) -> String {
    let created = call(state, "POST", path, vec![("uploads", None)], b"").await;
    assert_eq!(created.status, 200, "{}", created.body);
    text(&created.body, "UploadId").expect("an upload id")
}

async fn part(
    state: &tessari_s3_api::ApiState,
    path: &str,
    upload: &str,
    number: &str,
    bytes: &[u8],
) -> Seen {
    call(
        state,
        "PUT",
        path,
        vec![("partNumber", Some(number)), ("uploadId", Some(upload))],
        bytes,
    )
    .await
}

/// The data ids of an upload's part records.
async fn part_data(planter: &MetaPool) -> Vec<[u8; 16]> {
    let answers = planter
        .run("SELECT data FROM parts;", Vec::new())
        .await
        .expect("parts read");
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Vec::new();
    };
    let mut ids: Vec<[u8; 16]> = records
        .into_iter()
        .filter_map(|(_, record)| match record {
            Value::Object(fields) => match fields.get("data") {
                Some(Value::Uuid(id)) => Some(*id),
                _ => None,
            },
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn parts_are_uploaded_replaced_listed_and_dropped_by_an_abort() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/mpart", vec![], b"").await.status, 200);
    let upload = create(&state, "/mpart/big").await;
    assert_eq!(upload.len(), 32, "{upload}");
    let first = part(&state, "/mpart/big", &upload, "1", &body(64, 1)).await;
    assert_eq!(first.status, 200, "{}", first.body);
    let second = part(&state, "/mpart/big", &upload, "2", b"tail").await;
    assert_eq!(
        etag(&second),
        format!("\"{}\"", md5_hex(b"tail")),
        "the part's MD5"
    );
    let before = part_data(&planter).await;
    assert_eq!(before.len(), 2);
    let replaced = part(&state, "/mpart/big", &upload, "1", &body(64, 2)).await;
    assert_eq!(replaced.status, 200, "{}", replaced.body);
    assert_ne!(etag(&replaced), etag(&first), "a new body, a new ETag");
    let after = part_data(&planter).await;
    let dropped: Vec<[u8; 16]> = before
        .iter()
        .filter(|id| !after.contains(id))
        .copied()
        .collect();
    assert_eq!(dropped.len(), 1, "the replaced part's data left the parts");
    assert!(
        queued(&planter).await.contains(&dropped[0]),
        "and was queued for reclamation"
    );
    let listed = call(
        &state,
        "GET",
        "/mpart/big",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(listed.status, 200, "{}", listed.body);
    assert_eq!(texts(&listed.body, "Part", "PartNumber"), ["1", "2"]);
    assert_eq!(
        texts(&listed.body, "Part", "ETag"),
        [etag(&replaced), etag(&second)]
    );
    assert_eq!(texts(&listed.body, "Part", "Size"), ["64", "4"]);
    let object = call(&state, "GET", "/mpart/big", vec![], b"").await;
    assert_eq!(
        object.code.as_deref(),
        Some("NoSuchKey"),
        "parts are not an object"
    );
    let listing = call(&state, "GET", "/mpart", vec![("list-type", Some("2"))], b"").await;
    assert!(
        !listing.body.contains("<Key>"),
        "nor listed: {}",
        listing.body
    );
    let aborted = call(
        &state,
        "DELETE",
        "/mpart/big",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(aborted.status, 204, "{}", aborted.body);
    let queued_now = queued(&planter).await;
    assert!(
        after.iter().all(|id| queued_now.contains(id)),
        "every part's data queued by the abort"
    );
    assert!(part_data(&planter).await.is_empty());
    let late = part(&state, "/mpart/big", &upload, "3", b"late").await;
    assert_eq!(late.code.as_deref(), Some("NoSuchUpload"), "{}", late.body);
    let again = call(
        &state,
        "DELETE",
        "/mpart/big",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(again.code.as_deref(), Some("NoSuchUpload"));
}

fn md5_hex(bytes: &[u8]) -> String {
    let mut hashes = tessari_s3_core::objects::checksum::Hashes::new();
    hashes.update(bytes);
    hashes.finish().md5_hex
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_upload_answers_only_for_its_own_key_and_valid_part_numbers() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/bound", vec![], b"").await.status, 200);
    let upload = create(&state, "/bound/k").await;
    let refused = |seen: &Seen| (seen.status, seen.code.clone());
    let other_key = part(&state, "/bound/other", &upload, "1", b"x").await;
    assert_eq!(refused(&other_key), (404, Some("NoSuchUpload".to_owned())));
    let forged = part(&state, "/bound/k", &"0".repeat(32), "1", b"x").await;
    assert_eq!(refused(&forged), (404, Some("NoSuchUpload".to_owned())));
    let garbage = part(&state, "/bound/k", "not-an-id", "1", b"x").await;
    assert_eq!(refused(&garbage), (404, Some("NoSuchUpload".to_owned())));
    for number in ["0", "10001", "x"] {
        let seen = part(&state, "/bound/k", &upload, number, b"x").await;
        assert_eq!(
            refused(&seen),
            (400, Some("InvalidArgument".to_owned())),
            "{number}"
        );
    }
    let nowhere = call(&state, "POST", "/nobucket/k", vec![("uploads", None)], b"").await;
    assert_eq!(refused(&nowhere), (404, Some("NoSuchBucket".to_owned())));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn list_parts_pages_by_part_number_marker() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/pager", vec![], b"").await.status, 200);
    let upload = create(&state, "/pager/k").await;
    for number in ["3", "1", "2"] {
        assert_eq!(
            part(&state, "/pager/k", &upload, number, number.as_bytes())
                .await
                .status,
            200
        );
    }
    let first = call(
        &state,
        "GET",
        "/pager/k",
        vec![
            ("uploadId", Some(upload.as_str())),
            ("max-parts", Some("2")),
        ],
        b"",
    )
    .await;
    assert_eq!(texts(&first.body, "Part", "PartNumber"), ["1", "2"]);
    assert_eq!(text(&first.body, "IsTruncated").as_deref(), Some("true"));
    assert_eq!(
        text(&first.body, "NextPartNumberMarker").as_deref(),
        Some("2")
    );
    let rest = call(
        &state,
        "GET",
        "/pager/k",
        vec![
            ("uploadId", Some(upload.as_str())),
            ("part-number-marker", Some("2")),
        ],
        b"",
    )
    .await;
    assert_eq!(texts(&rest.body, "Part", "PartNumber"), ["3"]);
    assert_eq!(text(&rest.body, "IsTruncated").as_deref(), Some("false"));
    // A second upload's parts never appear in the first's listing, whichever id sorts first.
    let other = create(&state, "/pager/k").await;
    assert_eq!(
        part(&state, "/pager/k", &other, "1", b"x").await.status,
        200
    );
    for (id, expected) in [(&upload, vec!["1", "2", "3"]), (&other, vec!["1"])] {
        let listed = call(
            &state,
            "GET",
            "/pager/k",
            vec![("uploadId", Some(id.as_str()))],
            b"",
        )
        .await;
        assert_eq!(texts(&listed.body, "Part", "PartNumber"), expected, "{id}");
    }
}
