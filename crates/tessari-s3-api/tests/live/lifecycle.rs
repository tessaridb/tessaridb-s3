//! Bucket lifecycle configuration against a real node (S3 ref 08 M7): stored and read back, absent until set, removed
//! by DeleteBucketLifecycle, and refused — not accepted and ignored — where it asks for what this server does not do.

use crate::multipart::{text, texts};
use crate::{IGNORED, Seen, call, fresh};

const RULES: &str = "<LifecycleConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
<Rule><ID>logs</ID><Filter><Prefix>logs/</Prefix></Filter><Status>Enabled</Status>\
<Expiration><Days>30</Days></Expiration>\
<NoncurrentVersionExpiration><NoncurrentDays>7</NoncurrentDays><NewerNoncurrentVersions>2</NewerNoncurrentVersions>\
</NoncurrentVersionExpiration></Rule>\
<Rule><ID>uploads</ID><Filter></Filter><Status>Disabled</Status>\
<AbortIncompleteMultipartUpload><DaysAfterInitiation>3</DaysAfterInitiation></AbortIncompleteMultipartUpload></Rule>\
</LifecycleConfiguration>";

pub(crate) async fn lifecycle(state: &tessari_s3_api::ApiState, method: &str, body: &str) -> Seen {
    call(
        state,
        method,
        "/cycle",
        vec![("lifecycle", None)],
        body.as_bytes(),
    )
    .await
}

fn rule(body: &str, actions: &str) -> String {
    format!(
        "<LifecycleConfiguration><Rule><ID>r</ID><Filter>{body}</Filter><Status>Enabled</Status>{actions}</Rule>\
         </LifecycleConfiguration>"
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_lifecycle_configuration_is_stored_read_back_and_deleted() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    let none = lifecycle(&state, "GET", "").await;
    assert_eq!(none.status, 404, "{}", none.body);
    assert_eq!(none.code.as_deref(), Some("NoSuchLifecycleConfiguration"));
    let put = lifecycle(&state, "PUT", RULES).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let got = lifecycle(&state, "GET", "").await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(
        texts(&got.body, "Rule", "ID"),
        vec!["logs".to_owned(), "uploads".to_owned()]
    );
    assert_eq!(
        texts(&got.body, "Rule", "Status"),
        vec!["Enabled".to_owned(), "Disabled".to_owned()]
    );
    assert_eq!(text(&got.body, "Prefix").as_deref(), Some("logs/"));
    assert_eq!(text(&got.body, "Days").as_deref(), Some("30"));
    assert_eq!(text(&got.body, "NoncurrentDays").as_deref(), Some("7"));
    assert_eq!(
        text(&got.body, "NewerNoncurrentVersions").as_deref(),
        Some("2")
    );
    assert_eq!(text(&got.body, "DaysAfterInitiation").as_deref(), Some("3"));
    let deleted = lifecycle(&state, "DELETE", "").await;
    assert_eq!(deleted.status, 204, "{}", deleted.body);
    assert_eq!(lifecycle(&state, "GET", "").await.status, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn what_this_server_does_not_do_is_refused_and_a_bad_rule_is_named() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    let refusals = [
        (
            rule(
                "",
                "<Transition><Days>30</Days><StorageClass>GLACIER</StorageClass></Transition>",
            ),
            501,
            "NotImplemented",
        ),
        (
            rule(
                "<Tag><Key>k</Key><Value>v</Value></Tag>",
                "<Expiration><Days>1</Days></Expiration>",
            ),
            501,
            "NotImplemented",
        ),
        (
            rule("", "<Expiration><Days>0</Days></Expiration>"),
            400,
            "InvalidArgument",
        ),
        (
            rule("", "<Expiration><Days>x</Days></Expiration>"),
            400,
            "MalformedXML",
        ),
        ("<Nope/>".to_owned(), 400, "MalformedXML"),
    ];
    for (document, status, code) in refusals {
        let seen = lifecycle(&state, "PUT", &document).await;
        assert_eq!(seen.status, status, "{document}: {}", seen.body);
        assert_eq!(seen.code.as_deref(), Some(code), "{document}");
    }
    assert_eq!(
        lifecycle(&state, "GET", "").await.status,
        404,
        "nothing refused was stored"
    );
}

/// An instant `days` days after now, as the lifecycle pass is told the time.
pub(crate) fn days_ahead(days: i64) -> tessari_s3_types::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch");
    tessari_s3_types::Timestamp {
        seconds: days
            .checked_mul(86_400)
            .and_then(|ahead| i64::try_from(now.as_secs()).ok()?.checked_add(ahead))
            .expect("an instant days ahead fits"),
        nanos: 0,
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_rules_expire_versions_markers_and_uploads_and_a_repeated_pass_changes_nothing() {
    use crate::multipart::create;
    use crate::versions_kept::{put, set_versioning, versions};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    set_versioning(&state, "cycle", "Enabled").await;
    put(&state, "/cycle/logs/a", b"first").await;
    put(&state, "/cycle/logs/a", b"second").await;
    put(&state, "/cycle/keep/b", b"outside the prefix").await;
    put(&state, "/cycle/off/c", b"under a disabled rule").await;
    create(&state, "/cycle/logs/big").await;
    let rules = "<LifecycleConfiguration>\
        <Rule><ID>logs</ID><Filter><Prefix>logs/</Prefix></Filter><Status>Enabled</Status>\
        <Expiration><Days>1</Days></Expiration>\
        <NoncurrentVersionExpiration><NoncurrentDays>1</NoncurrentDays></NoncurrentVersionExpiration>\
        <AbortIncompleteMultipartUpload><DaysAfterInitiation>1</DaysAfterInitiation></AbortIncompleteMultipartUpload>\
        </Rule>\
        <Rule><ID>off</ID><Filter><Prefix>off/</Prefix></Filter><Status>Disabled</Status>\
        <Expiration><Days>1</Days></Expiration></Rule></LifecycleConfiguration>";
    assert_eq!(lifecycle(&state, "PUT", rules).await.status, 200);
    let lifecycle_runs = state.storage().lifecycle();
    let now = days_ahead(3);
    // Pass 1: the noncurrent `first` goes; the current `second` expires, leaving a marker over it; the upload aborts.
    lifecycle_runs.pass(now, 100, 0).await.expect("pass 1");
    let kept = versions(&planter, "cycle", "logs/a").await;
    assert_eq!(
        kept.iter()
            .map(|version| version.marker)
            .collect::<Vec<_>>(),
        vec![true, false],
        "a marker over `second`, and `first` gone"
    );
    let uploads = call(&state, "GET", "/cycle", vec![("uploads", None)], b"").await;
    assert!(
        !uploads.body.contains("<Upload>"),
        "the old upload was aborted: {}",
        uploads.body
    );
    // Pass 2 removes `second`, now noncurrent since the marker; pass 3 removes the marker, then its key's only version.
    lifecycle_runs.pass(now, 100, 0).await.expect("pass 2");
    lifecycle_runs.pass(now, 100, 0).await.expect("pass 3");
    assert!(
        versions(&planter, "cycle", "logs/a").await.is_empty(),
        "nothing of logs/a is left"
    );
    assert_eq!(
        versions(&planter, "cycle", "keep/b").await.len(),
        1,
        "outside the prefix"
    );
    assert_eq!(
        versions(&planter, "cycle", "off/c").await.len(),
        1,
        "a disabled rule does nothing"
    );
    let applied = lifecycle_runs.pass(now, 100, 0).await.expect("pass 4");
    assert_eq!(applied.actions(), 0, "a repeated pass changes nothing");
}

/// The current version of `bucket/key` as a listing shows it: its ETag and when it was written.
async fn current(
    state: &tessari_s3_api::ApiState,
    bucket: &str,
    key: &str,
) -> (String, tessari_s3_types::Timestamp) {
    use tessari_s3_core::objects::upload_listing::{UploadEntry, UploadSpec};
    use tessari_s3_storage::objects::{VersionKind, VersionsListed};
    let spec = UploadSpec {
        prefix: key.to_owned(),
        delimiter: None,
        max_uploads: 10,
        key_marker: None,
    };
    let name = tessari_s3_types::BucketName::new(bucket).expect("bucket name");
    let VersionsListed::Page(page) = state
        .storage()
        .objects()
        .list_versions(&name, spec, None)
        .await
        .expect("listed")
    else {
        panic!("the bucket lists");
    };
    match page.entries.into_iter().next() {
        Some(UploadEntry::Upload { item, .. }) => match item.kind {
            VersionKind::Object { etag, .. } => (etag, item.modified),
            other => panic!("the current version is an object, not {other:?}"),
        },
        other => panic!("one version, not {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_expiry_judged_on_an_earlier_write_spares_an_identical_rewrite() {
    use crate::versions_kept::put;
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/fence", vec![], b"").await.status, 200);
    put(&state, "/fence/k", b"same bytes").await;
    let (etag, judged) = current(&state, "fence", "k").await;
    // Rewritten with the same bytes after the pass judged it: the ETag is the same, the write is not.
    put(&state, "/fence/k", b"same bytes").await;
    let (again, rewritten) = current(&state, "fence", "k").await;
    assert_eq!(again, etag, "identical bytes keep the ETag");
    assert_ne!(rewritten, judged, "the rewrite is a later write");
    let key = tessari_s3_types::ObjectKey::new("k").expect("key");
    let bucket = tessari_s3_types::BucketName::new("fence").expect("bucket name");
    let objects = state.storage().objects();
    assert!(
        !objects
            .expire_current(&bucket, &key, &etag, judged)
            .await
            .expect("expiry ran"),
        "an expiry judged on the earlier write removes nothing"
    );
    assert_eq!(
        call(&state, "GET", "/fence/k", vec![], b"").await.status,
        200,
        "the rewrite is still there"
    );
    // Control: judged on the write that is current, the same call does expire it.
    assert!(
        objects
            .expire_current(&bucket, &key, &etag, rewritten)
            .await
            .expect("expiry ran")
    );
    assert_eq!(
        call(&state, "GET", "/fence/k", vec![], b"").await.status,
        404
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn newer_noncurrent_versions_are_kept_and_older_ones_removed() {
    use crate::versions_kept::{put, set_versioning, versions};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/cycle", vec![], b"").await.status, 200);
    set_versioning(&state, "cycle", "Enabled").await;
    for bytes in [b"one", b"two", b"six"] {
        put(&state, "/cycle/k", bytes).await;
    }
    let before = versions(&planter, "cycle", "k").await;
    let rules = "<LifecycleConfiguration><Rule><ID>keep</ID><Filter></Filter><Status>Enabled</Status>\
        <NoncurrentVersionExpiration><NoncurrentDays>1</NoncurrentDays>\
        <NewerNoncurrentVersions>1</NewerNoncurrentVersions></NoncurrentVersionExpiration></Rule>\
        </LifecycleConfiguration>";
    assert_eq!(lifecycle(&state, "PUT", rules).await.status, 200);
    let applied = state
        .storage()
        .lifecycle()
        .pass(days_ahead(3), 100, 0)
        .await
        .expect("pass");
    assert_eq!(
        applied.noncurrent, 1,
        "only the oldest noncurrent version goes"
    );
    let after = versions(&planter, "cycle", "k").await;
    assert_eq!(
        after
            .iter()
            .map(|kept| kept.version.clone())
            .collect::<Vec<_>>(),
        before
            .iter()
            .take(2)
            .map(|kept| kept.version.clone())
            .collect::<Vec<_>>(),
        "the current version and the newest noncurrent one stay"
    );
}
