//! Object Lock against a real node (S3 ref 08 M5; ADR-0016), what yields and to whom: GOVERNANCE to who administers
//! the bucket and asks for the bypass, a legal hold to its release, and a bucket without Object Lock refuses what it
//! could not keep.

use tessari_s3_core::authz::{Access, Role};
use tessari_s3_infrastructure::tessaridb::Value;

use crate::access::{grant, user_with_key};
use crate::delete_many::md5_of;
use crate::object_lock::{
    BYPASS, from_now, in_an_hour, locked_put, put_document, refused, version_record,
};
use crate::versions_delete::delete_version;
use crate::versions_kept::{set_versioning, version_header};
use crate::{IGNORED, call, call_as, call_with, fresh_with, scratch_dir};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn governance_yields_only_to_the_bucket_administrator_asking_for_the_bypass() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/docs", vec![], vec![], b"")
            .await
            .status,
        200
    );
    grant(
        storage,
        "bob",
        "docs",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    // Object Lock on an existing bucket needs its versioning enabled first.
    let early = put_document(&state, "/docs", "object-lock", "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled></ObjectLockConfiguration>", &[]).await;
    assert_eq!(refused(&early), (409, Some("InvalidBucketState")));
    set_versioning(&state, "docs", "Enabled").await;
    let configured = put_document(
        &state, "/docs", "object-lock",
        "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>GOVERNANCE</Mode><Days>1</Days></DefaultRetention></Rule></ObjectLockConfiguration>",
        &[],
    ).await;
    assert_eq!(configured.status, 200, "{}", configured.body);

    // The default applies to a write that names no retention — one that proves its bytes.
    assert_eq!(
        refused(&call(&state, "PUT", "/docs/k", vec![], b"draft").await),
        (400, Some("InvalidRequest")),
        "a retained write without a digest"
    );
    let md5 = md5_of("draft");
    let id = version_header(
        &call_with(
            &state,
            "PUT",
            "/docs/k",
            vec![],
            vec![("content-md5", md5.as_str())],
            b"draft",
        )
        .await,
    )
    .expect("an id");
    let retention = call(&state, "GET", "/docs/k", vec![("retention", None)], b"").await;
    assert!(
        retention.body.contains("<Mode>GOVERNANCE</Mode>"),
        "{}",
        retention.body
    );

    // A multipart object takes the bucket's default when it completes.
    let started = call(&state, "POST", "/docs/big", vec![("uploads", None)], b"").await;
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
        "/docs/big",
        vec![("partNumber", Some("1")), ("uploadId", Some(&upload))],
        b"one part",
    )
    .await;
    let etag = part.headers["etag"].to_str().expect("etag").to_owned();
    let done = call(
        &state,
        "POST",
        "/docs/big",
        vec![("uploadId", Some(&upload))],
        format!(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{etag}</ETag></Part>\
             </CompleteMultipartUpload>"
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let kept = call(&state, "GET", "/docs/big", vec![("retention", None)], b"").await;
    assert!(
        kept.body.contains("<Mode>GOVERNANCE</Mode>"),
        "{}",
        kept.body
    );

    // A writer may not bypass; the header alone does not bypass; administering and asking does.
    let writer = call_as(
        &state,
        bob.pair(),
        "DELETE",
        "/docs/k",
        vec![("versionId", Some(&id))],
        vec![BYPASS],
        b"",
    )
    .await;
    assert_eq!(refused(&writer), (403, Some("AccessDenied")));
    assert_eq!(
        refused(&delete_version(&state, "/docs/k", &id).await),
        (403, Some("AccessDenied"))
    );
    // A bypassed shortening is consumed by that one write: a later delete without the bypass is still refused.
    let half = from_now(1_800);
    let shorter = format!(
        "<Retention><Mode>GOVERNANCE</Mode><RetainUntilDate>{half}</RetainUntilDate></Retention>"
    );
    let md5 = md5_of(&shorter);
    let shortened = call_with(
        &state,
        "PUT",
        "/docs/k",
        vec![("retention", None), ("versionId", Some(&id))],
        vec![("content-md5", md5.as_str()), BYPASS],
        shorter.as_bytes(),
    )
    .await;
    assert_eq!(shortened.status, 200, "{}", shortened.body);
    assert_eq!(
        refused(&delete_version(&state, "/docs/k", &id).await),
        (403, Some("AccessDenied")),
        "the bypass did not outlive its write"
    );
    let record = version_record(&planter, "docs", "k", &id).await;
    let direct = planter
        .run(
            "DELETE versions:$target;",
            vec![("target".to_owned(), Value::Bytes(record))],
        )
        .await
        .expect_err("the store still keeps it");
    assert!(direct.to_string().contains("ObjectLocked:"), "{direct}");
    let removed = call_with(
        &state,
        "DELETE",
        "/docs/k",
        vec![("versionId", Some(&id))],
        vec![BYPASS],
        b"",
    )
    .await;
    assert_eq!(removed.status, 204, "{}", removed.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_legal_hold_keeps_a_version_until_it_is_released() {
    let _why = IGNORED;
    let (state, _, _) = fresh_with(scratch_dir()).await;
    let made = call_with(
        &state,
        "PUT",
        "/held",
        vec![],
        vec![("x-amz-bucket-object-lock-enabled", "true")],
        b"",
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let id = version_header(
        &locked_put(
            &state,
            "/held/k",
            "evidence",
            &[("x-amz-object-lock-legal-hold", "ON")],
        )
        .await,
    )
    .expect("an id");
    let hold = call(&state, "GET", "/held/k", vec![("legal-hold", None)], b"").await;
    assert!(hold.body.contains("<Status>ON</Status>"), "{}", hold.body);
    assert_eq!(
        refused(&delete_version(&state, "/held/k", &id).await),
        (403, Some("AccessDenied"))
    );
    let off = "<LegalHold><Status>OFF</Status></LegalHold>";
    let released = call_with(
        &state,
        "PUT",
        "/held/k",
        vec![("legal-hold", None)],
        vec![("content-md5", md5_of(off).as_str())],
        off.as_bytes(),
    )
    .await;
    assert_eq!(released.status, 200, "{}", released.body);
    assert_eq!(delete_version(&state, "/held/k", &id).await.status, 204);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_without_object_lock_refuses_what_it_could_not_keep() {
    let _why = IGNORED;
    let (state, _, _) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/plain", vec![], b"").await.status, 200);
    let until = in_an_hour();
    let md5 = md5_of("x");
    let asked = call_with(
        &state,
        "PUT",
        "/plain/k",
        vec![],
        vec![
            ("content-md5", md5.as_str()),
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            ("x-amz-object-lock-retain-until-date", &until),
        ],
        b"x",
    )
    .await;
    assert_eq!(refused(&asked), (400, Some("InvalidRequest")));
    let none = call(&state, "GET", "/plain", vec![("object-lock", None)], b"").await;
    assert_eq!(
        refused(&none),
        (404, Some("ObjectLockConfigurationNotFoundError"))
    );
    // A retention without a digest of the bytes is refused even in a lock bucket.
    assert_eq!(
        call_with(
            &state,
            "PUT",
            "/locked",
            vec![],
            vec![("x-amz-bucket-object-lock-enabled", "true")],
            b""
        )
        .await
        .status,
        200
    );
    let undigested = call_with(
        &state,
        "PUT",
        "/locked/k",
        vec![],
        vec![
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            ("x-amz-object-lock-retain-until-date", &until),
        ],
        b"x",
    )
    .await;
    assert_eq!(refused(&undigested), (400, Some("InvalidRequest")));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn governance_settings_are_only_extended_without_the_bypass() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    let lockable = vec![("x-amz-bucket-object-lock-enabled", "true")];
    assert_eq!(
        call_with(&state, "PUT", "/held", vec![], lockable, b"")
            .await
            .status,
        200
    );
    let until = in_an_hour();
    let put = locked_put(
        &state,
        "/held/k",
        "kept",
        &[
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            ("x-amz-object-lock-retain-until-date", until.as_str()),
        ],
    )
    .await;
    let id = version_header(&put).expect("a version id");
    let retention = |mode: &str, at: &str| {
        format!("<Retention><Mode>{mode}</Mode><RetainUntilDate>{at}</RetainUntilDate></Retention>")
    };
    // Governance keeps a version's lock settings from changing, except that its period may be extended.
    let moved = put_document(
        &state,
        "/held/k",
        "retention",
        &retention("COMPLIANCE", &until),
        &[],
    )
    .await;
    assert_eq!(
        refused(&moved),
        (403, Some("AccessDenied")),
        "{}",
        moved.body
    );
    let later = from_now(7200);
    let extended = put_document(
        &state,
        "/held/k",
        "retention",
        &retention("GOVERNANCE", &later),
        &[],
    )
    .await;
    assert_eq!(extended.status, 200, "{}", extended.body);
    // The store's own event holds the same line against a statement that goes around the server.
    let record = version_record(&planter, "held", "k", &id).await;
    let direct = planter
        .run(
            "UPDATE versions:$target SET lock_mode = 'COMPLIANCE';",
            vec![("target".to_owned(), Value::Bytes(record))],
        )
        .await
        .expect_err("the store refused the mode change");
    assert!(direct.to_string().contains("ObjectLocked:"), "{direct}");
    // Who administers the bucket and asks for the bypass may change it.
    let bypassed = put_document(
        &state,
        "/held/k",
        "retention",
        &retention("COMPLIANCE", &later),
        &[BYPASS],
    )
    .await;
    assert_eq!(bypassed.status, 200, "{}", bypassed.body);
    let read = call(&state, "GET", "/held/k", vec![("retention", None)], b"").await;
    assert!(
        read.body.contains("<Mode>COMPLIANCE</Mode>"),
        "{}",
        read.body
    );
}
