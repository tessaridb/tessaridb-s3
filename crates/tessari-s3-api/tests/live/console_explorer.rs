//! The console's bucket explorer, against a real node: a listing row says what each object IS — its content type as
//! stored at write, beside size, age and ETag — whichever path wrote it, so an operator sees a bucket's contents
//! without opening any object — the side pane's details come from the metadata record alone.

use std::sync::atomic::Ordering;

use serde_json::Value;
use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::console::{send, sign_in};
use crate::console_users::sign_in_as;
use crate::multipart::{etag, part, text};
use crate::{IGNORED, call, call_as, call_with, fresh_console, fresh_console_with, scratch_dir};

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

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_objects_details_show_its_retention_legal_hold_and_tags() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    let made = call_with(
        &state,
        "PUT",
        "/kept",
        vec![],
        vec![("x-amz-bucket-object-lock-enabled", "true")],
        b"",
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let until = crate::object_lock::in_an_hour();
    crate::object_lock::locked_put(
        &state,
        "/kept/doc.txt",
        "hello",
        &[
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            ("x-amz-object-lock-retain-until-date", &until),
            ("x-amz-object-lock-legal-hold", "ON"),
            ("x-amz-tagging", "team=blue&tier=gold"),
        ],
    )
    .await;
    assert_eq!(call(&state, "PUT", "/plain", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/plain/doc.txt", vec![], b"x")
            .await
            .status,
        200
    );

    let detail = |path: &'static str| send(&console, "GET", path, Some(&root), None);
    let kept = detail("/api/v1/buckets/kept/object?key=doc.txt").await;
    assert_eq!(kept.status, 200, "{:?}", kept.json());
    let shown = kept.json();
    assert_eq!(shown["retention"]["mode"], Value::from("GOVERNANCE"));
    assert_eq!(shown["retention"]["until"], Value::from(until.as_str()));
    assert_eq!(shown["legal_hold"], Value::from(true));
    assert_eq!(shown["tags"]["team"], Value::from("blue"));
    assert_eq!(shown["tags"]["tier"], Value::from("gold"));

    let plain = detail("/api/v1/buckets/plain/object?key=doc.txt").await;
    assert_eq!(plain.status, 200, "{:?}", plain.json());
    let shown = plain.json();
    assert_eq!(shown["retention"], Value::Null);
    assert_eq!(shown["legal_hold"], Value::from(false));
    assert_eq!(shown["tags"], serde_json::json!({}));
}

/// Writes `keys` into `bucket`, a bounded number at a time.
async fn put_many(state: &tessari_s3_api::ApiState, bucket: &str, keys: &[String]) {
    for chunk in keys.chunks(32) {
        let mut writes = tokio::task::JoinSet::new();
        for key in chunk {
            let (state, path) = (state.clone(), format!("/{bucket}/{key}"));
            writes.spawn(async move {
                let seen = call(&state, "PUT", &path, vec![], b"x").await;
                (seen.status, seen.body)
            });
        }
        while let Some(done) = writes.join_next().await {
            let (status, body) = done.expect("the write ran");
            assert_eq!(status, 200, "{body}");
        }
    }
}

/// One console listing page: its keys, its rolled-up prefixes and where the next page starts.
async fn page(
    console: &tessari_s3_api::ConsoleState,
    root: &str,
    prefix: &str,
    limit: usize,
    cursor: Option<&str>,
) -> (Vec<String>, Vec<String>, Option<String>) {
    // This test's keys and prefixes hold only digits, letters, `.` and `/`, all legal in a query as they are.
    let mut path =
        format!("/api/v1/buckets/many/objects?delimiter=%2F&limit={limit}&prefix={prefix}");
    if let Some(cursor) = cursor {
        path.push_str("&cursor=");
        path.push_str(cursor);
    }
    let seen = send(console, "GET", &path, Some(root), None).await;
    assert_eq!(seen.status, 200, "{:?}", seen.json());
    let body = seen.json();
    let texts = |value: &Value, field: Option<&str>| -> Vec<String> {
        value
            .as_array()
            .expect("an array")
            .iter()
            .map(|item| {
                field
                    .map_or(item, |name| &item[name])
                    .as_str()
                    .expect("text")
                    .to_owned()
            })
            .collect()
    };
    (
        texts(&body["objects"], Some("key")),
        texts(&body["prefixes"], None),
        body["next"].as_str().map(str::to_owned),
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn five_thousand_keys_scroll_by_cursor_with_none_repeated_or_skipped() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/many", vec![], b"").await.status, 200);
    let mut keys: Vec<String> = (0..5_000).map(|n| format!("{n:05}.bin")).collect();
    keys.extend((0..30).map(|n| format!("f{n:02}/inside")));
    put_many(&state, "many", &keys).await;

    let (capped, _, more) = page(&console, &root, "", 5_000, None).await;
    assert_eq!(capped.len(), 1_000, "a page never exceeds the console cap");
    assert!(more.is_some());

    // Walk 250 at a time; after the third page, change the bucket on both sides of the cursor.
    let (mut listed, mut folders, mut cursor) = (Vec::new(), Vec::new(), None);
    let mut pages = 0;
    loop {
        let (objects, prefixes, next) = page(&console, &root, "", 250, cursor.as_deref()).await;
        assert!(objects.len() + prefixes.len() <= 250);
        listed.extend(objects);
        folders.extend(prefixes);
        pages += 1;
        if pages == 3 {
            for (method, key) in [
                ("DELETE", "00100.bin"),
                ("DELETE", "04000.bin"),
                ("PUT", "00100a.bin"),
                ("PUT", "04000a.bin"),
            ] {
                assert!(
                    call(&state, method, &format!("/many/{key}"), vec![], b"x")
                        .await
                        .status
                        < 300
                );
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut sorted = listed.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(listed, sorted, "keys arrive in byte order, each once");
    let steady: Vec<&String> = keys
        .iter()
        .filter(|key| !key.contains('/') && *key != "00100.bin" && *key != "04000.bin")
        .collect();
    let missing: Vec<&&String> = steady.iter().filter(|key| !listed.contains(key)).collect();
    assert!(
        missing.is_empty(),
        "keys present throughout were skipped: {missing:?}"
    );
    assert!(
        listed.iter().any(|key| key == "04000a.bin"),
        "a key written ahead of the cursor is reached"
    );
    assert!(
        !listed.iter().any(|key| key == "04000.bin"),
        "a key deleted ahead of the cursor is not"
    );
    let expected: Vec<String> = (0..30).map(|n| format!("f{n:02}/")).collect();
    assert_eq!(folders, expected, "every folder rolls up once");
    assert!(pages >= 20, "the walk took {pages} pages");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_name_filter_is_a_prefix_the_store_lists_and_pages_within() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/many", vec![], b"").await.status, 200);
    let mut keys: Vec<String> = (0..300).map(|n| format!("a/{n}")).collect();
    keys.extend((0..50).map(|n| format!("b/{n}")));
    keys.push("a.txt".to_owned());
    put_many(&state, "many", &keys).await;

    // Inside folder `a/`, names starting `1`: a/1, a/10..a/19, a/100..a/199.
    let mut expected: Vec<String> = keys
        .iter()
        .filter(|key| key.starts_with("a/1"))
        .cloned()
        .collect();
    expected.sort();
    assert_eq!(expected.len(), 111);
    let (mut listed, mut cursor, mut pages) = (Vec::new(), None, 0);
    loop {
        let (objects, prefixes, next) = page(&console, &root, "a/1", 40, cursor.as_deref()).await;
        assert!(prefixes.is_empty());
        listed.extend(objects);
        pages += 1;
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        listed, expected,
        "exactly the names that start so, in byte order, each once"
    );
    assert!(pages >= 3, "the filtered set paged: {pages} pages");

    // At the top, names starting `a`: the folder `a/` rolls up and `a.txt` is listed; nothing under `b/`.
    let (objects, prefixes, next) = page(&console, &root, "a", 100, None).await;
    assert_eq!(
        (objects, prefixes, next),
        (vec!["a.txt".to_owned()], vec!["a/".to_owned()], None)
    );

    let (objects, prefixes, next) = page(&console, &root, "zz", 100, None).await;
    assert!(
        objects.is_empty() && prefixes.is_empty() && next.is_none(),
        "no name starts so"
    );
}

/// Sends each of `paths` as `token`; the ones at `refused` must answer 403 with none of `leaks` in the body.
async fn refused_saying_nothing(
    console: &tessari_s3_api::ConsoleState,
    token: &str,
    paths: &[(&str, String, Option<&str>)],
    refused: &[usize],
    leaks: &[&str],
) {
    for (index, (method, path, body)) in paths.iter().enumerate() {
        let seen = send(console, method, path, Some(token), *body).await;
        if !refused.contains(&index) {
            continue;
        }
        let text = String::from_utf8_lossy(&seen.bytes);
        assert_eq!(seen.status, 403, "{method} {path}: {text}");
        for leak in leaks {
            assert!(
                !text.contains(leak),
                "{method} {path} leaked {leak}: {text}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_explorer_answers_nothing_about_a_bucket_its_caller_may_not_read() {
    let _why = IGNORED;
    let (state, console, now) = crate::console_users::states().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let carl = user_with_key(storage, "carl", "beta", Role::SpaceAdmin, true).await;
    for (path, headers) in [
        ("/ann-photos", vec![]),
        (
            "/ann-photos/secret-plan.png",
            vec![("content-type", "image/x-secret")],
        ),
    ] {
        let put = call_as(&state, ann.pair(), "PUT", path, vec![], headers, b"x").await;
        assert_eq!(put.status, 200, "{}", put.body);
    }
    // A grant across spaces reaches nothing; the crossing is attempted with one in place.
    grant(
        storage,
        "carl",
        "ann-photos",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    let ann_token = sign_in_as(&console, &ann).await.expect("ann");
    let bob_token = sign_in_as(&console, &bob).await.expect("bob");
    let carl_token = sign_in_as(&console, &carl).await.expect("carl");
    let detail = send(
        &console,
        "GET",
        "/api/v1/buckets/ann-photos/object?key=secret-plan.png",
        Some(&ann_token),
        None,
    )
    .await;
    assert_eq!(detail.status, 200, "the control: the owner reads it");
    let etag = detail.json()["etag"].as_str().expect("an etag").to_owned();
    let delete_body = format!(
        r#"{{"etag":{},"reason":"probe"}}"#,
        Value::from(etag.as_str())
    );

    let paths = [
        ("GET", "/api/v1/buckets/ann-photos/objects".to_owned(), None),
        (
            "GET",
            "/api/v1/buckets/ann-photos/objects?prefix=secret&delimiter=%2F&limit=5".to_owned(),
            None,
        ),
        (
            "GET",
            "/api/v1/buckets/ann-photos/objects?cursor=key%3Aa&limit=5".to_owned(),
            None,
        ),
        (
            "GET",
            "/api/v1/buckets/ann-photos/object?key=secret-plan.png".to_owned(),
            None,
        ),
        (
            "GET",
            "/api/v1/buckets/ann-photos/object/content?key=secret-plan.png".to_owned(),
            None,
        ),
        (
            "DELETE",
            "/api/v1/buckets/ann-photos/object?key=secret-plan.png".to_owned(),
            Some(delete_body.as_str()),
        ),
        // Absent objects and buckets answer exactly as a present one does.
        (
            "GET",
            "/api/v1/buckets/ann-photos/object?key=never-written".to_owned(),
            None,
        ),
        (
            "GET",
            "/api/v1/buckets/no-such-bucket/objects".to_owned(),
            None,
        ),
        (
            "GET",
            "/api/v1/buckets/no-such-bucket/object?key=secret-plan.png".to_owned(),
            None,
        ),
    ];
    let every: Vec<usize> = (0..paths.len()).collect();
    let leaks = ["secret-plan", "image/x-secret", etag.as_str()];
    refused_saying_nothing(&console, &carl_token, &paths, &every, &leaks).await;
    refused_saying_nothing(&console, &bob_token, &paths, &every, &leaks).await;

    // A read grant opens the reads in its own space, and still not the delete.
    grant(
        storage,
        "bob",
        "ann-photos",
        Access {
            read: true,
            write: false,
        },
    )
    .await;
    now.fetch_add(5, Ordering::AcqRel);
    for path in [
        "/api/v1/buckets/ann-photos/objects?prefix=secret",
        "/api/v1/buckets/ann-photos/object?key=secret-plan.png",
        "/api/v1/buckets/ann-photos/object/content?key=secret-plan.png",
    ] {
        let seen = send(&console, "GET", path, Some(&bob_token), None).await;
        assert_eq!(seen.status, 200, "{path}");
    }
    refused_saying_nothing(&console, &bob_token, &paths, &[5, 7, 8], &leaks).await;

    let kept = send(
        &console,
        "GET",
        "/api/v1/buckets/ann-photos/object?key=secret-plan.png",
        Some(&ann_token),
        None,
    )
    .await;
    assert_eq!(kept.status, 200, "no refused delete removed the object");
    assert_eq!(kept.json()["etag"], Value::from(etag.as_str()));
}
