//! A copy of a named version (S3 ref 05, ref 07 M7): `x-amz-copy-source: bucket/key?versionId=…` copies that version,
//! whether or not it is current, and answers `x-amz-copy-source-version-id`; a delete marker named by id is no source
//! (400), a current one is as if the key were gone (404), an id naming nothing is `NoSuchVersion`; the version's read
//! is asked as `s3:GetObjectVersion`.

use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::complete::{complete, document};
use crate::multipart::{create, text};
use crate::policy::put_policy;
use crate::versions_kept::{put, set_versioning, version_header};
use crate::{IGNORED, Seen, call, call_as, call_with, fresh};

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

async fn copy_as(state: &tessari_s3_api::ApiState, pair: (&str, &str), source: &str) -> Seen {
    let headers = vec![("x-amz-copy-source", source)];
    call_as(state, pair, "PUT", "/cva/copy", vec![], headers, b"").await
}

async fn copy(state: &tessari_s3_api::ApiState, to: &str, source: &str) -> Seen {
    call_with(
        state,
        "PUT",
        to,
        vec![],
        vec![("x-amz-copy-source", source)],
        b"",
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_named_version_is_copied_whether_or_not_it_is_current() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cvv", vec![], b"").await.status, 200);
    set_versioning(&state, "cvv", "Enabled").await;
    let first = version_header(&put(&state, "/cvv/k", b"first").await).expect("an id");
    let second = version_header(&put(&state, "/cvv/k", b"second").await).expect("an id");
    let copied = copy(&state, "/cvv/old", &format!("cvv/k?versionId={first}")).await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    assert_eq!(
        header(&copied, "x-amz-copy-source-version-id"),
        Some(first.as_str())
    );
    assert_eq!(
        call(&state, "GET", "/cvv/old", vec![], b"").await.bytes,
        b"first"
    );
    let current = copy(&state, "/cvv/new", "cvv/k").await;
    assert_eq!(
        header(&current, "x-amz-copy-source-version-id"),
        Some(second.as_str())
    );
    assert_eq!(
        call(&state, "GET", "/cvv/new", vec![], b"").await.bytes,
        b"second"
    );

    let deleted = call(&state, "DELETE", "/cvv/k", vec![], b"").await;
    let marker = version_header(&deleted).expect("a marker id");
    let gone = copy(&state, "/cvv/x", "cvv/k").await;
    assert_eq!(gone.code.as_deref(), Some("NoSuchKey"), "{}", gone.body);
    let named_marker = copy(&state, "/cvv/x", &format!("cvv/k?versionId={marker}")).await;
    assert_eq!(named_marker.status, 400, "{}", named_marker.body);
    assert_eq!(named_marker.code.as_deref(), Some("InvalidRequest"));
    let kept = copy(&state, "/cvv/kept", &format!("cvv/k?versionId={second}")).await;
    assert_eq!(kept.status, 200, "{}", kept.body);
    assert_eq!(
        call(&state, "GET", "/cvv/kept", vec![], b"").await.bytes,
        b"second"
    );
    let nothing = copy(
        &state,
        "/cvv/x",
        "cvv/k?versionId=00000000000000000000000000000001",
    )
    .await;
    assert!(
        matches!(
            nothing.code.as_deref(),
            Some("NoSuchVersion" | "InvalidArgument")
        ),
        "{}",
        nothing.body
    );
    assert_eq!(call(&state, "GET", "/cvv/x", vec![], b"").await.status, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_part_is_copied_from_a_named_version() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cvp", vec![], b"").await.status, 200);
    set_versioning(&state, "cvp", "Enabled").await;
    let first = version_header(&put(&state, "/cvp/k", b"first part").await).expect("an id");
    put(&state, "/cvp/k", b"later").await;
    let upload = create(&state, "/cvp/joined").await;
    let source = format!("cvp/k?versionId={first}");
    let part = call_with(
        &state,
        "PUT",
        "/cvp/joined",
        vec![("partNumber", Some("1")), ("uploadId", Some(&upload))],
        vec![("x-amz-copy-source", &source)],
        b"",
    )
    .await;
    assert_eq!(part.status, 200, "{}", part.body);
    assert_eq!(
        header(&part, "x-amz-copy-source-version-id"),
        Some(first.as_str())
    );
    let tag = text(&part.body, "ETag").expect("an ETag");
    let done = complete(
        &state,
        "/cvp/joined",
        &upload,
        vec![],
        &document(&[("1", &tag)]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(
        call(&state, "GET", "/cvp/joined", vec![], b"").await.bytes,
        b"first part"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_is_copied_only_by_who_may_read_versions() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "cvspace", Role::Member, true).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/cva", vec![], vec![], b"")
            .await
            .status,
        200
    );
    set_versioning(&state, "cva", "Enabled").await;
    let first = version_header(&put(&state, "/cva/k", b"first").await).expect("an id");
    let deny = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Deny","Principal":{"AWS":"arn:aws:iam::cvspace:user/ann"},"Action":"s3:GetObjectVersion","Resource":"arn:aws:s3:::cva/*"}]}"#;
    let set = put_policy(&state, "cva", deny).await;
    assert!(set.status == 200 || set.status == 204, "{}", set.body);
    let named = copy_as(&state, ann.pair(), &format!("cva/k?versionId={first}")).await;
    assert_eq!(
        named.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        named.body
    );
    let plain = copy_as(&state, ann.pair(), "cva/k").await;
    assert_eq!(plain.status, 200, "{}", plain.body);
}
