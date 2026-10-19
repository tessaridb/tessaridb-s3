//! Object Lock against a real node (S3 ref 08 M5, M6; ADR-0016): a retained or held version survives every path that
//! could release it — a version delete, a batch entry, lifecycle, suspending versioning, deleting the bucket, and a
//! statement written straight against the store — while a simple delete only adds a marker; COMPLIANCE yields to nobody.
//! What does yield, and to whom, is `object_lock_yielding`.

use tessari_s3_api::ApiState;
use tessari_s3_infrastructure::tessaridb::{MetaPool, Value};
use tessari_s3_types::{BucketName, ObjectKey, Timestamp};

use crate::delete_many::md5_of;
use crate::versions_delete::delete_version;
use crate::versions_kept::{version_header, versions};
use crate::{IGNORED, Seen, call, call_with, live_config, prepared, scratch_dir, test_node};

pub(crate) const BYPASS: (&str, &str) = ("x-amz-bypass-governance-retention", "true");

/// The instant `seconds` from now, as an S3 date.
pub(crate) fn from_now(seconds: i64) -> String {
    Timestamp {
        seconds: Timestamp::now()
            .seconds
            .checked_add(seconds)
            .expect("a near instant"),
        nanos: 0,
    }
    .iso8601_millis()
}

pub(crate) fn in_an_hour() -> String {
    from_now(3_600)
}

pub(crate) async fn locked_put(
    state: &tessari_s3_api::ApiState,
    path: &str,
    body: &str,
    extra: &[(&str, &str)],
) -> Seen {
    let md5 = md5_of(body);
    let mut headers = vec![("content-md5", md5.as_str())];
    headers.extend_from_slice(extra);
    let seen = call_with(state, "PUT", path, vec![], headers, body.as_bytes()).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    seen
}

pub(crate) async fn put_document(
    state: &tessari_s3_api::ApiState,
    path: &str,
    query: &str,
    document: &str,
    extra: &[(&str, &str)],
) -> Seen {
    let md5 = md5_of(document);
    let mut headers = vec![("content-md5", md5.as_str())];
    headers.extend_from_slice(extra);
    call_with(
        state,
        "PUT",
        path,
        vec![(query, None)],
        headers,
        document.as_bytes(),
    )
    .await
}

/// The record id of version `id` of `bucket/key`, derived from its documented layout: bucket, `/`, key, `00 01`, the
/// sequence inverted big-endian.
pub(crate) async fn version_record(
    planter: &MetaPool,
    bucket: &str,
    key: &str,
    id: &str,
) -> Vec<u8> {
    let seq = versions(planter, bucket, key)
        .await
        .into_iter()
        .find(|version| version.version == id)
        .expect("kept")
        .seq;
    let mut record = format!("{bucket}/{key}").into_bytes();
    record.extend_from_slice(&[0x00, 0x01]);
    record.extend_from_slice(&(!u64::try_from(seq).expect("a sequence")).to_be_bytes());
    record
}

pub(crate) fn refused(seen: &Seen) -> (u16, Option<&str>) {
    (seen.status, seen.code.as_deref())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_compliance_version_survives_every_path_that_could_release_it() {
    let _why = IGNORED;
    let config = live_config(test_node(), None, &scratch_dir());
    let state = ApiState::new(&config, ApiState::system_clock(), prepared(&config).await);
    // Buckets live in the namespace's global database, versions in the region's.
    let global = MetaPool::new(config.meta.clone()).expect("pool");
    let planter = global
        .regional(&config.region)
        .expect("the region's database");
    let made = call_with(
        &state,
        "PUT",
        "/vault",
        vec![],
        vec![("x-amz-bucket-object-lock-enabled", "true")],
        b"",
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let versioning = call(&state, "GET", "/vault", vec![("versioning", None)], b"").await;
    assert!(
        versioning.body.contains("<Status>Enabled</Status>"),
        "{}",
        versioning.body
    );
    let configuration = call(&state, "GET", "/vault", vec![("object-lock", None)], b"").await;
    assert!(
        configuration
            .body
            .contains("<ObjectLockEnabled>Enabled</ObjectLockEnabled>"),
        "{}",
        configuration.body
    );

    let until = in_an_hour();
    let id = version_header(
        &locked_put(
            &state,
            "/vault/k",
            "kept",
            &[
                ("x-amz-object-lock-mode", "COMPLIANCE"),
                ("x-amz-object-lock-retain-until-date", &until),
            ],
        )
        .await,
    )
    .expect("an id");
    let retention = call(
        &state,
        "GET",
        "/vault/k",
        vec![("retention", None), ("versionId", Some(&id))],
        b"",
    )
    .await;
    assert!(
        retention.body.contains("<Mode>COMPLIANCE</Mode>"),
        "{}",
        retention.body
    );

    // A version delete, even with the bypass, and a batch entry naming it.
    assert_eq!(
        refused(&delete_version(&state, "/vault/k", &id).await),
        (403, Some("AccessDenied"))
    );
    let bypassed = call_with(
        &state,
        "DELETE",
        "/vault/k",
        vec![("versionId", Some(&id))],
        vec![BYPASS],
        b"",
    )
    .await;
    assert_eq!(
        refused(&bypassed),
        (403, Some("AccessDenied")),
        "COMPLIANCE yields to nobody"
    );
    let batch =
        format!("<Delete><Object><Key>k</Key><VersionId>{id}</VersionId></Object></Delete>");
    let md5 = md5_of(&batch);
    let answered = call_with(
        &state,
        "POST",
        "/vault",
        vec![("delete", None)],
        vec![("content-md5", md5.as_str())],
        batch.as_bytes(),
    )
    .await;
    assert!(
        answered.body.contains("<Code>AccessDenied</Code>"),
        "{}",
        answered.body
    );

    // Lifecycle expiring it, suspending versioning, deleting the bucket.
    let vault = BucketName::new("vault").expect("name");
    let key = ObjectKey::new("k").expect("key");
    let expired = state
        .storage()
        .objects()
        .expire_version(&vault, &key, &id)
        .await
        .expect("answers");
    assert_eq!(expired, tessari_s3_storage::objects::VersionRemoved::Locked);
    let suspend = put_document(
        &state,
        "/vault",
        "versioning",
        "<VersioningConfiguration><Status>Suspended</Status></VersioningConfiguration>",
        &[],
    )
    .await;
    assert_eq!(refused(&suspend), (409, Some("InvalidBucketState")));

    // A simple delete only adds a marker; the bucket then holds versions and is not empty.
    let marked = call(&state, "DELETE", "/vault/k", vec![], b"").await;
    assert_eq!(marked.status, 204);
    assert_eq!(
        refused(&call(&state, "DELETE", "/vault", vec![], b"").await),
        (409, Some("BucketNotEmpty"))
    );

    // Retention is extended by a writer and never shortened.
    let longer = from_now(7_200);
    let extend = call_with(
        &state, "PUT", "/vault/k", vec![("retention", None), ("versionId", Some(&id))],
        vec![("content-md5", md5_of(&format!("<Retention><Mode>COMPLIANCE</Mode><RetainUntilDate>{longer}</RetainUntilDate></Retention>")).as_str())],
        format!("<Retention><Mode>COMPLIANCE</Mode><RetainUntilDate>{longer}</RetainUntilDate></Retention>").as_bytes(),
    ).await;
    assert_eq!(extend.status, 200, "{}", extend.body);
    let shorten = call_with(
        &state, "PUT", "/vault/k", vec![("retention", None), ("versionId", Some(&id))],
        vec![("content-md5", md5_of(&format!("<Retention><Mode>COMPLIANCE</Mode><RetainUntilDate>{until}</RetainUntilDate></Retention>")).as_str()), BYPASS],
        format!("<Retention><Mode>COMPLIANCE</Mode><RetainUntilDate>{until}</RetainUntilDate></Retention>").as_bytes(),
    ).await;
    assert_eq!(refused(&shorten), (403, Some("AccessDenied")));

    // No path around the server either: a statement straight against the store is refused by its own event.
    let direct = planter
        .run(
            "DELETE FROM versions WHERE bucket_name = $bucket LIMIT ALL;",
            vec![("bucket".to_owned(), Value::String("vault".to_owned()))],
        )
        .await;
    let refusal = direct.expect_err("the store refused releasing a retained version");
    assert!(
        refusal.to_string().contains("ObjectLocked:"),
        "refused by its own event: {refusal}"
    );
    assert!(
        versions(&planter, "vault", "k")
            .await
            .iter()
            .any(|version| version.version == id),
        "the retained version is still there"
    );
    let record = version_record(&planter, "vault", "k", &id).await;
    let bound = || {
        vec![
            ("target".to_owned(), Value::Bytes(record.clone())),
            ("bucket".to_owned(), Value::String("vault".to_owned())),
        ]
    };
    for (statement, what) in [
        (
            "UPDATE versions:$target SET retain_until = NONE;",
            "ObjectLocked:",
        ),
        (
            "UPDATE versions:$target SET lock_mode = 'GOVERNANCE';",
            "ObjectLocked:",
        ),
        (
            "UPDATE buckets:$bucket SET versioning = 'Suspended';",
            "ObjectLockKept:",
        ),
        (
            "UPDATE buckets:$bucket SET object_lock = NONE;",
            "ObjectLockKept:",
        ),
    ] {
        let pool = if statement.contains("buckets:") {
            &global
        } else {
            &planter
        };
        let refusal = pool.run(statement, bound()).await.expect_err(statement);
        assert!(refusal.to_string().contains(what), "{statement}: {refusal}");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_multipart_upload_carries_the_lock_its_create_asked_for() {
    let _why = IGNORED;
    let (state, _) = crate::fresh().await;
    let lockable = vec![("x-amz-bucket-object-lock-enabled", "true")];
    assert_eq!(
        call_with(&state, "PUT", "/vault", vec![], lockable, b"")
            .await
            .status,
        200
    );
    assert_eq!(call(&state, "PUT", "/plain", vec![], b"").await.status, 200);
    let until = in_an_hour();
    let asked = vec![
        ("x-amz-object-lock-mode", "GOVERNANCE"),
        ("x-amz-object-lock-retain-until-date", until.as_str()),
        ("x-amz-object-lock-legal-hold", "ON"),
    ];

    // A bucket without Object Lock refuses the headers by name, as PutObject does.
    let plain = call_with(
        &state,
        "POST",
        "/plain/big",
        vec![("uploads", None)],
        asked.clone(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&plain),
        (400, Some("InvalidRequest")),
        "{}",
        plain.body
    );
    assert!(
        plain.body.contains("Object Lock Configuration"),
        "{}",
        plain.body
    );

    let created = call_with(
        &state,
        "POST",
        "/vault/big",
        vec![("uploads", None)],
        asked,
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = crate::multipart::text(&created.body, "UploadId").expect("an upload id");
    let uploaded =
        crate::multipart::part(&state, "/vault/big", &upload, "1", b"locked bytes").await;
    assert_eq!(uploaded.status, 200, "{}", uploaded.body);
    let tag = crate::multipart::etag(&uploaded);
    let document = crate::complete::document(&[("1", tag.as_str())]);
    let done = crate::complete::complete(&state, "/vault/big", &upload, vec![], &document).await;
    assert_eq!(done.status, 200, "{}", done.body);
    let id = version_header(&done).expect("a version id");

    let retention = call(&state, "GET", "/vault/big", vec![("retention", None)], b"").await;
    assert!(
        retention.body.contains("<Mode>GOVERNANCE</Mode>"),
        "{}",
        retention.body
    );
    let hold = call(&state, "GET", "/vault/big", vec![("legal-hold", None)], b"").await;
    assert!(hold.body.contains("<Status>ON</Status>"), "{}", hold.body);
    // Held: not even the governance bypass removes it.
    let removed = call_with(
        &state,
        "DELETE",
        "/vault/big",
        vec![("versionId", Some(id.as_str()))],
        vec![BYPASS],
        b"",
    )
    .await;
    assert_eq!(
        refused(&removed),
        (403, Some("AccessDenied")),
        "{}",
        removed.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_read_of_a_locked_version_answers_its_lock() {
    let _why = IGNORED;
    let (state, _) = crate::fresh().await;
    let lockable = vec![("x-amz-bucket-object-lock-enabled", "true")];
    assert_eq!(
        call_with(&state, "PUT", "/vault", vec![], lockable, b"")
            .await
            .status,
        200
    );
    let until = in_an_hour();
    locked_put(
        &state,
        "/vault/k",
        "kept",
        &[
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            ("x-amz-object-lock-retain-until-date", until.as_str()),
            ("x-amz-object-lock-legal-hold", "ON"),
        ],
    )
    .await;
    locked_put(&state, "/vault/free", "free", &[]).await;
    let header = |seen: &Seen, name: &str| {
        seen.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    for method in ["HEAD", "GET"] {
        let read = call(&state, method, "/vault/k", vec![], b"").await;
        assert_eq!(read.status, 200, "{}", read.body);
        assert_eq!(
            header(&read, "x-amz-object-lock-mode").as_deref(),
            Some("GOVERNANCE"),
            "{method}"
        );
        assert_eq!(
            header(&read, "x-amz-object-lock-retain-until-date"),
            Some(until.clone()),
            "{method}"
        );
        assert_eq!(
            header(&read, "x-amz-object-lock-legal-hold").as_deref(),
            Some("ON"),
            "{method}"
        );
        let free = call(&state, method, "/vault/free", vec![], b"").await;
        assert_eq!(
            header(&free, "x-amz-object-lock-mode"),
            None,
            "{method}: nothing kept, nothing answered"
        );
    }
}
