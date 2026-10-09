//! What a write and a delete leave behind, per versioning state, against a real node: an enabled bucket keeps every
//! version and its bytes, a suspended one replaces only the null version, a never-versioned one keeps exactly one
//! version per key, and the versions of one key stay in write order under concurrent writers.

use tessari_s3_infrastructure::tessaridb::{MetaPool, Number, Value};

use crate::large::{files, records};
use crate::multipart::{create, etag, part, part_data};
use crate::{IGNORED, Seen, call, fresh_with, scratch_dir};

/// One stored version of a key: its id, sequence, whether it is a delete marker, and its data file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Kept {
    pub(crate) version: String,
    pub(crate) seq: i64,
    pub(crate) marker: bool,
    pub(crate) data: Option<[u8; 16]>,
}

/// The versions of `key` in `bucket`, in the order the store keeps them (newest first).
pub(crate) async fn versions(planter: &MetaPool, bucket: &str, key: &str) -> Vec<Kept> {
    let parameters = vec![
        ("bucket".to_owned(), Value::String(bucket.to_owned())),
        ("key".to_owned(), Value::String(key.to_owned())),
    ];
    let script = "SELECT version_id, seq, marker, data FROM versions WHERE bucket_name = $bucket AND key = $key;";
    records(planter, script, parameters)
        .await
        .into_iter()
        .map(|record| {
            let Value::Object(fields) = record else {
                panic!("a version is an object: {record:?}");
            };
            Kept {
                version: match fields.get("version_id") {
                    Some(Value::String(id)) => id.clone(),
                    other => panic!("version_id: {other:?}"),
                },
                seq: match fields.get("seq") {
                    Some(Value::Number(Number::Integer(seq))) => *seq,
                    other => panic!("seq: {other:?}"),
                },
                marker: matches!(fields.get("marker"), Some(Value::Bool(true))),
                data: match fields.get("data") {
                    Some(Value::Uuid(id)) => Some(*id),
                    _ => None,
                },
            }
        })
        .collect()
}

pub(crate) async fn set_versioning(state: &tessari_s3_api::ApiState, bucket: &str, status: &str) {
    let body =
        format!("<VersioningConfiguration><Status>{status}</Status></VersioningConfiguration>");
    let path = format!("/{bucket}");
    let seen = call(
        state,
        "PUT",
        &path,
        vec![("versioning", None)],
        body.as_bytes(),
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
}

pub(crate) fn version_header(seen: &Seen) -> Option<String> {
    seen.headers
        .get("x-amz-version-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(crate) async fn put(state: &tessari_s3_api::ApiState, path: &str, bytes: &[u8]) -> Seen {
    let seen = call(state, "PUT", path, vec![], bytes).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    seen
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_enabled_bucket_keeps_every_version_and_its_bytes() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    set_versioning(&state, "files", "Enabled").await;

    let mut issued = Vec::new();
    for body in [&b"one"[..], b"two", b"three"] {
        let id = version_header(&put(&state, "/files/k", body).await).expect("a version id");
        assert_eq!(id.len(), 32, "{id}");
        issued.push(id);
    }
    let kept = versions(&planter, "files", "k").await;
    let ids: Vec<&str> = kept
        .iter()
        .map(|version| version.version.as_str())
        .collect();
    issued.reverse();
    assert_eq!(ids, issued, "newest first, each the id its PUT answered");
    assert_eq!(
        kept.iter().map(|version| version.seq).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert!(
        kept.iter()
            .all(|version| !version.marker && version.data.is_some())
    );

    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(files(&dir).len(), 3, "no version's file is reclaimed");
    let current = call(&state, "GET", "/files/k", vec![], b"").await;
    assert_eq!(current.body, "three");
    assert_eq!(version_header(&current), Some(issued[0].clone()));
    let head = call(&state, "HEAD", "/files/k", vec![], b"").await;
    assert_eq!(version_header(&head), Some(issued[0].clone()));

    // A delete adds a marker and removes nothing.
    let deleted = call(&state, "DELETE", "/files/k", vec![], b"").await;
    assert_eq!(deleted.status, 204, "{}", deleted.body);
    let kept = versions(&planter, "files", "k").await;
    assert_eq!(kept.len(), 4);
    assert!(kept[0].marker && kept[0].data.is_none() && kept[0].seq == 4);
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.status,
        404
    );
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(files(&dir).len(), 3, "a delete marker releases nothing");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_never_versioned_bucket_keeps_one_null_version_per_key() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    assert_eq!(version_header(&put(&state, "/files/k", b"one").await), None);
    put(&state, "/files/k", b"two").await;
    let current = call(&state, "GET", "/files/k", vec![], b"").await;
    assert_eq!(current.body, "two");
    assert_eq!(version_header(&current), None, "never versioned: no id");
    let kept = versions(&planter, "files", "k").await;
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].version, "null");
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(files(&dir).len(), 1, "the overwritten bytes are reclaimed");

    assert_eq!(
        call(&state, "DELETE", "/files/k", vec![], b"").await.status,
        204
    );
    assert!(
        versions(&planter, "files", "k").await.is_empty(),
        "nothing is left of the key"
    );
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert!(files(&dir).is_empty(), "the deleted bytes are reclaimed");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_suspended_bucket_replaces_only_the_null_version() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    put(&state, "/files/k", b"before").await;
    set_versioning(&state, "files", "Enabled").await;
    put(&state, "/files/k", b"enabled").await;
    set_versioning(&state, "files", "Suspended").await;
    assert_eq!(
        version_header(&put(&state, "/files/k", b"first null").await).as_deref(),
        Some("null")
    );
    put(&state, "/files/k", b"second null").await;

    let kept = versions(&planter, "files", "k").await;
    let shape: Vec<(&str, i64)> = kept
        .iter()
        .map(|version| (version.version.as_str(), version.seq))
        .collect();
    assert_eq!(shape.len(), 2, "{kept:?}");
    assert_eq!(
        shape[0],
        ("null", 4),
        "the null version written last is the newest"
    );
    assert_eq!(shape[1].1, 2, "the version written while enabled stays");
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(
        files(&dir).len(),
        2,
        "both replaced null versions' bytes are reclaimed"
    );

    // A delete replaces the null version with a null marker and keeps the rest.
    assert_eq!(
        call(&state, "DELETE", "/files/k", vec![], b"").await.status,
        204
    );
    let kept = versions(&planter, "files", "k").await;
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(kept[0].marker && kept[0].version == "null");
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(files(&dir).len(), 1);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_noncurrent_multipart_version_keeps_its_parts() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    set_versioning(&state, "files", "Enabled").await;
    let upload = create(&state, "/files/big").await;
    let first = part(&state, "/files/big", &upload, "1", b"only part").await;
    let document = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part></CompleteMultipartUpload>",
        etag(&first)
    );
    let done = call(
        &state,
        "POST",
        "/files/big",
        vec![("uploadId", Some(upload.as_str()))],
        document.as_bytes(),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let parts = part_data(&planter).await;
    assert_eq!(parts.len(), 1);

    put(&state, "/files/big", b"small now").await;
    assert_eq!(
        part_data(&planter).await,
        parts,
        "the noncurrent version still owns its part"
    );
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(part_data(&planter).await, parts);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn concurrent_writers_of_one_key_leave_versions_in_write_order() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    set_versioning(&state, "files", "Enabled").await;
    let mut writes = tokio::task::JoinSet::new();
    for writer in 0..16_u8 {
        let state = state.clone();
        writes.spawn(async move { call(&state, "PUT", "/files/hot", vec![], &[writer]).await });
    }
    let mut answered = Vec::new();
    while let Some(done) = writes.join_next().await {
        let seen = done.expect("the write ran");
        assert_eq!(seen.status, 200, "{}", seen.body);
        answered.push(version_header(&seen).expect("a version id"));
    }
    let kept = versions(&planter, "files", "hot").await;
    let seqs: Vec<i64> = kept.iter().map(|version| version.seq).collect();
    assert_eq!(
        seqs,
        (1..=16).rev().collect::<Vec<i64>>(),
        "one version per write, numbered in commit order"
    );
    let mut ids: Vec<String> = kept.iter().map(|version| version.version.clone()).collect();
    ids.sort();
    answered.sort();
    assert_eq!(ids, answered, "every answered id is kept");
    let current = call(&state, "GET", "/files/hot", vec![], b"").await;
    let newest = version_header(&current).expect("the current version's id");
    assert_eq!(
        newest, kept[0].version,
        "the object read is the newest version"
    );
}
