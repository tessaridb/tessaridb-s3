//! The console's bucket explorer, against a real node: a listing row says what each object IS — its content type as
//! stored at write, beside size, age and ETag — whichever path wrote it, so an operator sees a bucket's contents
//! without opening any object.

use serde_json::Value;

use crate::console::{send, sign_in};
use crate::multipart::{etag, part, text};
use crate::{IGNORED, call, call_with, fresh_console};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listing_row_carries_the_content_type_each_write_path_stored() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    let put = |path: &'static str, headers: Vec<(&'static str, &'static str)>| {
        call_with(&state, "PUT", path, vec![], headers, b"0123456789")
    };
    assert_eq!(
        put("/files/photo.png", vec![("content-type", "image/png")])
            .await
            .status,
        200
    );
    assert_eq!(put("/files/plain", vec![]).await.status, 200);
    let created = call_with(
        &state,
        "POST",
        "/files/doc.txt",
        vec![("uploads", None)],
        vec![("content-type", "text/plain")],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let first = part(&state, "/files/doc.txt", &upload, "1", b"hello").await;
    let document = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part>\
         </CompleteMultipartUpload>",
        etag(&first)
    );
    let done = call(
        &state,
        "POST",
        "/files/doc.txt",
        vec![("uploadId", Some(upload.as_str()))],
        document.as_bytes(),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let copied = put(
        "/files/copy.json",
        vec![
            ("x-amz-copy-source", "files/photo.png"),
            ("x-amz-metadata-directive", "REPLACE"),
            ("content-type", "application/json"),
        ],
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);

    let listed = send(
        &console,
        "GET",
        "/api/v1/buckets/files/objects",
        Some(&root),
        None,
    )
    .await;
    assert_eq!(listed.status, 200, "{:?}", listed.json());
    let rows = listed.json()["objects"].clone();
    let type_of = |key: &str| {
        rows.as_array()
            .expect("rows")
            .iter()
            .find(|row| row["key"] == key)
            .unwrap_or_else(|| panic!("{key} listed: {rows}"))["content_type"]
            .clone()
    };
    assert_eq!(type_of("photo.png"), Value::from("image/png"));
    assert_eq!(
        type_of("plain"),
        Value::Null,
        "a write that named no type lists none"
    );
    assert_eq!(type_of("doc.txt"), Value::from("text/plain"));
    assert_eq!(type_of("copy.json"), Value::from("application/json"));
}
