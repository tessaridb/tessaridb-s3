//! How tags travel (S3 ref 05 M8, M9): CopyObject's tagging directive, a multipart upload's tags at completion, a
//! bucket's own tags, and a retained version's tags still free to change while its bytes are kept.

use crate::delete_many::md5_of;
use crate::object_lock::{from_now, refused};
use crate::tagging::{document, get_tags, listed, owned, put_tags};
use crate::{IGNORED, call, call_with, fresh};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_copy_keeps_or_replaces_tags_and_a_multipart_object_takes_its_uploads() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/tags", vec![], b"").await.status, 200);
    let tagged = call_with(
        &state,
        "PUT",
        "/tags/src",
        vec![],
        vec![("x-amz-tagging", "from=source")],
        b"bytes",
    )
    .await;
    assert_eq!(tagged.status, 200, "{}", tagged.body);
    let copy = |destination: &'static str, extra: Vec<(&'static str, &'static str)>| {
        let state = &state;
        async move {
            let mut headers = vec![("x-amz-copy-source", "/tags/src")];
            headers.extend(extra);
            call_with(state, "PUT", destination, vec![], headers, b"").await
        }
    };
    // A bucket never versioned names no version, as its reads name none.
    let untagged = put_tags(
        &state,
        "/tags/src",
        vec![],
        &document(&[("from", "source")]),
    )
    .await;
    assert_eq!(untagged.status, 200, "{}", untagged.body);
    assert!(untagged.headers.get("x-amz-version-id").is_none());
    assert_eq!(copy("/tags/kept", vec![]).await.status, 200);
    assert_eq!(
        listed(&get_tags(&state, "/tags/kept", vec![]).await),
        owned(&[("from", "source")])
    );
    let replaced = copy(
        "/tags/new",
        vec![
            ("x-amz-tagging-directive", "REPLACE"),
            ("x-amz-tagging", "from=request"),
        ],
    )
    .await;
    assert_eq!(replaced.status, 200, "{}", replaced.body);
    assert_eq!(
        listed(&get_tags(&state, "/tags/new", vec![]).await),
        owned(&[("from", "request")])
    );

    let started = call_with(
        &state,
        "POST",
        "/tags/big",
        vec![("uploads", None)],
        vec![("x-amz-tagging", "made=parts")],
        b"",
    )
    .await;
    let upload = started
        .body
        .split("<UploadId>")
        .nth(1)
        .and_then(|rest| rest.split("</UploadId>").next())
        .expect("an upload id")
        .to_owned();
    let part = call(
        &state,
        "PUT",
        "/tags/big",
        vec![("partNumber", Some("1")), ("uploadId", Some(&upload))],
        b"one part",
    )
    .await;
    let etag = part.headers["etag"].to_str().expect("etag").to_owned();
    let done = call(
        &state, "POST", "/tags/big", vec![("uploadId", Some(&upload))],
        format!("<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{etag}</ETag></Part></CompleteMultipartUpload>").as_bytes(),
    ).await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(
        listed(&get_tags(&state, "/tags/big", vec![]).await),
        owned(&[("made", "parts")])
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_has_its_own_tags_and_a_retained_version_its_tags_free() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/labelled", vec![], b"").await.status,
        200
    );
    let none = call(&state, "GET", "/labelled", vec![("tagging", None)], b"").await;
    assert_eq!(refused(&none), (404, Some("NoSuchTagSet")));
    let body = document(&[("cost", "team-a"), ("env", "prod")]);
    let undigested = call(
        &state,
        "PUT",
        "/labelled",
        vec![("tagging", None)],
        body.as_bytes(),
    )
    .await;
    assert_eq!(refused(&undigested), (400, Some("InvalidRequest")));
    let put = put_tags(&state, "/labelled", vec![], &body).await;
    assert_eq!(put.status, 204, "{}", put.body);
    let read = call(&state, "GET", "/labelled", vec![("tagging", None)], b"").await;
    assert_eq!(listed(&read), owned(&[("cost", "team-a"), ("env", "prod")]));
    let fifty_one: Vec<(String, String)> =
        (0..51).map(|n| (format!("k{n}"), String::new())).collect();
    let fifty_one: Vec<(&str, &str)> = fifty_one
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        refused(&put_tags(&state, "/labelled", vec![], &document(&fifty_one)).await),
        (400, Some("InvalidTag"))
    );
    let dropped = call(&state, "DELETE", "/labelled", vec![("tagging", None)], b"").await;
    assert_eq!(dropped.status, 204, "{}", dropped.body);
    assert_eq!(
        refused(&call(&state, "GET", "/labelled", vec![("tagging", None)], b"").await),
        (404, Some("NoSuchTagSet"))
    );

    // Object Lock keeps a version's bytes, not its tags.
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
    let until = from_now(3_600);
    let md5 = md5_of("held");
    let locked = call_with(
        &state,
        "PUT",
        "/kept/k",
        vec![],
        vec![
            ("content-md5", md5.as_str()),
            ("x-amz-object-lock-mode", "COMPLIANCE"),
            ("x-amz-object-lock-retain-until-date", until.as_str()),
        ],
        b"held",
    )
    .await;
    assert_eq!(locked.status, 200, "{}", locked.body);
    let retagged = put_tags(&state, "/kept/k", vec![], &document(&[("reviewed", "yes")])).await;
    assert_eq!(retagged.status, 200, "{}", retagged.body);
    assert_eq!(
        listed(&get_tags(&state, "/kept/k", vec![]).await),
        owned(&[("reviewed", "yes")])
    );
    assert_eq!(
        call(&state, "GET", "/kept/k", vec![], b"").await.bytes,
        b"held"
    );
}
