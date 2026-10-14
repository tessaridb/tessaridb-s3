//! Deleting by version id against a real node (S3 ref 08 M3): a noncurrent version goes and nothing else changes, the
//! current one goes and the next newest becomes current, a delete marker goes and the object is back; a simple delete
//! answers the marker it added; a malformed id is refused, an unknown one is already gone, and only who administers the
//! bucket deletes a version.

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::large::files;
use crate::versions_kept::{put, set_versioning, version_header, versions};
use crate::{IGNORED, Seen, call, call_as, fresh_with, scratch_dir};

fn delete_marker(seen: &Seen) -> bool {
    seen.headers
        .get("x-amz-delete-marker")
        .is_some_and(|value| value.as_bytes() == b"true")
}

pub(crate) async fn delete_version(
    state: &tessari_s3_api::ApiState,
    path: &str,
    version: &str,
) -> Seen {
    call(
        state,
        "DELETE",
        path,
        vec![("versionId", Some(version))],
        b"",
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_deleted_by_id_goes_and_the_next_newest_becomes_current() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    set_versioning(&state, "files", "Enabled").await;
    let mut ids = Vec::new();
    for body in [&b"one"[..], b"two", b"three"] {
        ids.push(version_header(&put(&state, "/files/k", body).await).expect("an id"));
    }

    // A noncurrent version: it goes, its file is reclaimed, the current object is untouched.
    let gone = delete_version(&state, "/files/k", &ids[1]).await;
    assert_eq!(gone.status, 204, "{}", gone.body);
    assert_eq!(version_header(&gone).as_deref(), Some(ids[1].as_str()));
    assert!(!delete_marker(&gone));
    let kept: Vec<String> = versions(&planter, "files", "k")
        .await
        .into_iter()
        .map(|version| version.version)
        .collect();
    assert_eq!(kept, vec![ids[2].clone(), ids[0].clone()]);
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.body,
        "three"
    );
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(
        files(&dir).len(),
        2,
        "the deleted version's file is reclaimed"
    );

    // The current version: the next newest is what a read answers now.
    assert_eq!(
        delete_version(&state, "/files/k", &ids[2]).await.status,
        204
    );
    let read = call(&state, "GET", "/files/k", vec![], b"").await;
    assert_eq!(read.body, "one");
    assert_eq!(version_header(&read).as_deref(), Some(ids[0].as_str()));

    // A simple delete answers the marker it added; deleting that marker brings the object back.
    let marked = call(&state, "DELETE", "/files/k", vec![], b"").await;
    assert_eq!(marked.status, 204);
    assert!(delete_marker(&marked), "the answer names the marker");
    let marker = version_header(&marked).expect("the marker's id");
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.status,
        404
    );
    let undeleted = delete_version(&state, "/files/k", &marker).await;
    assert_eq!(undeleted.status, 204);
    assert!(
        delete_marker(&undeleted),
        "the removed version was a marker"
    );
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.body,
        "one"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_id_is_checked_and_the_null_version_is_deleted_by_name() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    put(&state, "/files/k", b"never versioned").await;
    let malformed = delete_version(&state, "/files/k", "not-a-version").await;
    assert_eq!(
        (malformed.status, malformed.code.as_deref()),
        (400, Some("InvalidArgument"))
    );
    let unknown = delete_version(&state, "/files/k", "00000000000000090000000000000009").await;
    assert_eq!(
        unknown.status, 204,
        "a version that is not there is already gone"
    );
    assert_eq!(
        versions(&planter, "files", "k").await.len(),
        1,
        "nothing else went"
    );

    let null = delete_version(&state, "/files/k", "null").await;
    assert_eq!(null.status, 204, "{}", null.body);
    assert!(versions(&planter, "files", "k").await.is_empty());
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.status,
        404
    );

    // Suspended: a simple delete adds a null marker.
    set_versioning(&state, "files", "Suspended").await;
    put(&state, "/files/s", b"suspended").await;
    let marked = call(&state, "DELETE", "/files/s", vec![], b"").await;
    assert!(delete_marker(&marked));
    assert_eq!(version_header(&marked).as_deref(), Some("null"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn only_who_administers_a_bucket_deletes_a_version() {
    let _why = IGNORED;
    let (state, _, _) = fresh_with(scratch_dir()).await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let made = call_as(&state, ann.pair(), "PUT", "/files", vec![], vec![], b"").await;
    assert_eq!(made.status, 200, "{}", made.body);
    grant(
        storage,
        "bob",
        "files",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    set_versioning(&state, "files", "Enabled").await;
    let id = version_header(&put(&state, "/files/k", b"kept").await).expect("an id");
    let query = || vec![("versionId", Some(id.as_str()))];

    let writer = call_as(
        &state,
        bob.pair(),
        "DELETE",
        "/files/k",
        query(),
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (writer.status, writer.code.as_deref()),
        (403, Some("AccessDenied")),
        "writing objects is not destroying their history"
    );
    let simple = call_as(
        &state,
        bob.pair(),
        "DELETE",
        "/files/k",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        simple.status, 204,
        "a writer still deletes the current object"
    );
    let owner = call_as(
        &state,
        ann.pair(),
        "DELETE",
        "/files/k",
        query(),
        vec![],
        b"",
    )
    .await;
    assert_eq!(owner.status, 204, "{}", owner.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_batch_deletes_versions_by_id_and_reports_each_key_as_s3_does() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/files", vec![], vec![], b"")
            .await
            .status,
        200
    );
    grant(
        storage,
        "bob",
        "files",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    set_versioning(&state, "files", "Enabled").await;
    let old = version_header(&put(&state, "/files/k", b"old").await).expect("an id");
    put(&state, "/files/k", b"new").await;
    put(&state, "/files/m", b"m").await;
    let document = format!(
        "<Delete><Object><Key>k</Key><VersionId>{old}</VersionId></Object>\
         <Object><Key>m</Key></Object>\
         <Object><Key>k</Key><VersionId>bad</VersionId></Object></Delete>"
    );

    // A writer: the simple delete goes ahead, the version entries are refused one by one and remove nothing.
    let md5 = crate::delete_many::md5_of(&document);
    let by_writer = call_as(
        &state,
        bob.pair(),
        "POST",
        "/files",
        vec![("delete", None)],
        vec![("content-md5", md5.as_str())],
        document.as_bytes(),
    )
    .await;
    assert_eq!(by_writer.status, 200, "{}", by_writer.body);
    assert!(
        by_writer.body.contains(&format!(
            "<Error><Key>k</Key><VersionId>{old}</VersionId><Code>AccessDenied</Code>"
        )),
        "{}",
        by_writer.body
    );
    assert!(
        by_writer.body.contains(
            "<Deleted><Key>m</Key><DeleteMarker>true</DeleteMarker><DeleteMarkerVersionId>"
        )
    );
    assert_eq!(
        versions(&planter, "files", "k").await.len(),
        2,
        "nothing of `k` went"
    );

    let by_owner = crate::delete_many::delete_many(&state, "files", &document).await;
    assert_eq!(by_owner.status, 200, "{}", by_owner.body);
    assert!(
        by_owner.body.contains(&format!(
            "<Deleted><Key>k</Key><VersionId>{old}</VersionId></Deleted>"
        )),
        "{}",
        by_owner.body
    );
    assert!(
        by_owner
            .body
            .contains("<Error><Key>k</Key><VersionId>bad</VersionId><Code>InvalidArgument</Code>"),
        "{}",
        by_owner.body
    );
    assert_eq!(
        versions(&planter, "files", "k").await.len(),
        1,
        "the named version went"
    );
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.body,
        "new"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn version_deletes_racing_writes_leave_the_newest_version_current() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    set_versioning(&state, "files", "Enabled").await;
    let mut old = Vec::new();
    for body in [&b"a"[..], b"b", b"c", b"d"] {
        old.push(version_header(&put(&state, "/files/hot", body).await).expect("an id"));
    }
    let mut racing = tokio::task::JoinSet::new();
    for id in old {
        let state = state.clone();
        racing.spawn(async move { delete_version(&state, "/files/hot", &id).await.status });
    }
    for writer in 0..8_u8 {
        let state = state.clone();
        racing.spawn(async move {
            call(&state, "PUT", "/files/hot", vec![], &[b'0' + writer])
                .await
                .status
        });
    }
    while let Some(done) = racing.join_next().await {
        let status = done.expect("ran");
        assert!(
            status == 200 || status == 204,
            "a write answers 200, a delete 204: {status}"
        );
    }
    let kept = versions(&planter, "files", "hot").await;
    assert_eq!(
        kept.len(),
        8,
        "every old version went, every write stayed: {kept:?}"
    );
    let current = call(&state, "GET", "/files/hot", vec![], b"").await;
    assert_eq!(
        version_header(&current).as_deref(),
        Some(kept[0].version.as_str()),
        "the object read is the newest version kept"
    );
}
