//! Reads by version against a real node (S3 ref 08 M4): GetObject and HeadObject with `versionId` answer that
//! version, a delete marker that is current answers 404 naming the marker, a `versionId` naming a marker answers 405,
//! and an id naming nothing answers `NoSuchVersion`.

use crate::complete::multipart_put;
use crate::large::body;
use crate::versions_kept::{put, set_versioning, version_header};
use crate::{IGNORED, Seen, call, fresh};

const MIB: usize = 1 << 20;

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

async fn read(state: &tessari_s3_api::ApiState, method: &str, path: &str, version: &str) -> Seen {
    call(state, method, path, vec![("versionId", Some(version))], b"").await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_reads_back_by_its_id_beside_the_current_object() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vread", vec![], b"").await.status, 200);
    set_versioning(&state, "vread", "Enabled").await;
    let first = version_header(&put(&state, "/vread/k", b"first").await).expect("an id");
    let second = version_header(&put(&state, "/vread/k", b"second, longer").await).expect("an id");
    let old = read(&state, "GET", "/vread/k", &first).await;
    assert_eq!(old.status, 200, "{}", old.body);
    assert_eq!(old.bytes, b"first");
    assert_eq!(header(&old, "x-amz-version-id"), Some(first.as_str()));
    let head = read(&state, "HEAD", "/vread/k", &first).await;
    assert_eq!(head.status, 200);
    assert_eq!(header(&head, "content-length"), Some("5"));
    assert_eq!(header(&head, "x-amz-version-id"), Some(first.as_str()));
    let current = call(&state, "GET", "/vread/k", vec![], b"").await;
    assert_eq!(current.bytes, b"second, longer");
    assert_eq!(header(&current, "x-amz-version-id"), Some(second.as_str()));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_id_naming_no_version_is_no_such_version_and_a_malformed_one_is_refused() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vnone", vec![], b"").await.status, 200);
    set_versioning(&state, "vnone", "Enabled").await;
    let kept = version_header(&put(&state, "/vnone/k", b"kept").await).expect("an id");
    // The same shape as a real id with its last character changed names no version.
    let mut other = kept.clone();
    let last = other.pop().expect("an id is not empty");
    other.push(if last == '0' { '1' } else { '0' });
    let missing = read(&state, "GET", "/vnone/k", &other).await;
    assert_eq!(missing.status, 404, "{}", missing.body);
    assert_eq!(missing.code.as_deref(), Some("NoSuchVersion"));
    assert_eq!(read(&state, "HEAD", "/vnone/k", &other).await.status, 404);
    let malformed = read(&state, "GET", "/vnone/k", "not a version id").await;
    assert_eq!(malformed.status, 400, "{}", malformed.body);
    assert_eq!(malformed.code.as_deref(), Some("InvalidArgument"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_current_delete_marker_answers_404_naming_it_and_a_read_of_it_by_id_405() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vmark", vec![], b"").await.status, 200);
    set_versioning(&state, "vmark", "Enabled").await;
    let object =
        version_header(&put(&state, "/vmark/k", b"before the marker").await).expect("an id");
    let deleted = call(&state, "DELETE", "/vmark/k", vec![], b"").await;
    assert_eq!(deleted.status, 204);
    let marker = version_header(&deleted).expect("the marker's id");
    for method in ["GET", "HEAD"] {
        let current = call(&state, method, "/vmark/k", vec![], b"").await;
        assert_eq!(current.status, 404, "{method}");
        assert_eq!(
            header(&current, "x-amz-delete-marker"),
            Some("true"),
            "{method}"
        );
        assert_eq!(
            header(&current, "x-amz-version-id"),
            Some(marker.as_str()),
            "{method}"
        );
        let by_id = read(&state, method, "/vmark/k", &marker).await;
        assert_eq!(by_id.status, 405, "{method}: {}", by_id.body);
        assert_eq!(
            header(&by_id, "x-amz-delete-marker"),
            Some("true"),
            "{method}"
        );
        assert!(header(&by_id, "last-modified").is_some(), "{method}");
    }
    let before = read(&state, "GET", "/vmark/k", &object).await;
    assert_eq!(before.status, 200);
    assert_eq!(before.bytes, b"before the marker");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_null_version_and_a_noncurrent_multipart_version_read_whole() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/vparts", vec![], b"").await.status,
        200
    );
    put(&state, "/vparts/k", b"written before versioning").await;
    set_versioning(&state, "vparts", "Enabled").await;
    let (first, tail) = (body(5 * MIB, 3), b"tail".to_vec());
    multipart_put(&state, "/vparts/k", &[&first, &tail]).await;
    let multipart = version_header(&call(&state, "HEAD", "/vparts/k", vec![], b"").await)
        .expect("the multipart version's id");
    put(&state, "/vparts/k", b"newest").await;
    let null = read(&state, "GET", "/vparts/k", "null").await;
    assert_eq!(null.status, 200, "{}", null.body);
    assert_eq!(null.bytes, b"written before versioning");
    let parts = read(&state, "GET", "/vparts/k", &multipart).await;
    assert_eq!(parts.status, 200, "{}", parts.body);
    assert!(
        parts.bytes == [first, tail].concat(),
        "the multipart version reads whole"
    );
}
