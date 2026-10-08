//! The console's bucket explorer, against a real node: a listing row says what each object IS — its content type as
//! stored at write, beside size, age and ETag — whichever path wrote it, so an operator sees a bucket's contents
//! without opening any object — the side pane's details come from the metadata record alone.

use serde_json::Value;

use crate::console::{send, sign_in};
use crate::multipart::{etag, part, text};
use crate::{IGNORED, call, call_with, fresh_console, fresh_console_with, scratch_dir};

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

/// Every regular file under `dir`.
fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(at) = pending.pop() {
        for entry in std::fs::read_dir(&at).expect("readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_objects_details_come_from_its_record_with_no_byte_of_it_read() {
    let _why = IGNORED;
    let (state, console, data_dir) = fresh_console_with(scratch_dir()).await;
    let root = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    let put = call_with(
        &state,
        "PUT",
        "/files/doc.txt",
        vec![],
        vec![("content-type", "text/plain"), ("x-amz-meta-owner", "ann")],
        b"hello",
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);

    let written = files_under(&data_dir);
    assert!(!written.is_empty(), "the object has a data file");
    for file in &written {
        std::fs::remove_file(file).expect("data file removed");
    }
    let read = call(&state, "GET", "/files/doc.txt", vec![], b"").await;
    assert_ne!(
        read.status, 200,
        "the bytes are gone, so a read of them fails: {}",
        read.body
    );

    let detail = send(
        &console,
        "GET",
        "/api/v1/buckets/files/object?key=doc.txt",
        Some(&root),
        None,
    )
    .await;
    assert_eq!(detail.status, 200, "{:?}", detail.json());
    let shown = detail.json();
    assert_eq!(shown["size"], Value::from(5));
    assert_eq!(shown["headers"]["content-type"], Value::from("text/plain"));
    assert_eq!(shown["metadata"]["owner"], Value::from("ann"));
    assert_eq!(shown["parts"], Value::Null);
    std::fs::remove_dir_all(&data_dir).expect("scratch removed");
}
