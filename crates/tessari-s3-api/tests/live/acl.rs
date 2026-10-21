//! ACLs stay disabled (Object Ownership BucketOwnerEnforced, S3 ref 04): reading a bucket's or an object's ACL
//! answers the owner's FULL_CONTROL, writing one is accepted only as `private` or `bucket-owner-full-control` and
//! changes nothing, and the ownership controls always read BucketOwnerEnforced.

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::{ACCESS_KEY, IGNORED, Seen, call, call_as, call_with, fresh};

async fn acl(
    state: &tessari_s3_api::ApiState,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
) -> Seen {
    let mut query = query;
    query.push(("acl", None));
    call(state, "GET", path, query, b"").await
}

async fn put_acl(
    state: &tessari_s3_api::ApiState,
    path: &str,
    headers: Vec<(&str, &str)>,
    body: &[u8],
) -> Seen {
    call_with(state, "PUT", path, vec![("acl", None)], headers, body).await
}

fn assert_owner_full_control(seen: &Seen) {
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert!(seen.body.contains("<AccessControlPolicy"), "{}", seen.body);
    assert!(
        seen.body.contains(&format!("<Owner><ID>{ACCESS_KEY}</ID>")),
        "{}",
        seen.body
    );
    assert!(
        seen.body.contains("xsi:type=\"CanonicalUser\""),
        "{}",
        seen.body
    );
    assert_eq!(seen.body.matches("<Grant>").count(), 1, "{}", seen.body);
    assert!(
        seen.body.contains("<Permission>FULL_CONTROL</Permission>"),
        "{}",
        seen.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_acl_reads_the_owner_and_takes_only_owner_acls() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/acls", vec![], b"").await.status, 200);
    assert_owner_full_control(&acl(&state, "/acls", vec![]).await);
    for canned in ["private", "bucket-owner-full-control"] {
        let seen = put_acl(&state, "/acls", vec![("x-amz-acl", canned)], b"").await;
        assert_eq!(seen.status, 200, "{canned}: {}", seen.body);
    }
    for headers in [
        vec![("x-amz-acl", "public-read")],
        vec![("x-amz-grant-read", "id=someone")],
    ] {
        let seen = put_acl(&state, "/acls", headers, b"").await;
        assert_eq!(seen.status, 400, "{}", seen.body);
        assert_eq!(seen.code.as_deref(), Some("AccessControlListNotSupported"));
    }
    let body = br#"<AccessControlPolicy xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><AccessControlList/></AccessControlPolicy>"#;
    let seen = put_acl(&state, "/acls", vec![], body).await;
    assert_eq!(
        seen.code.as_deref(),
        Some("AccessControlListNotSupported"),
        "{}",
        seen.body
    );
    // Nothing a write was accepted for changed what reads back.
    assert_owner_full_control(&acl(&state, "/acls", vec![]).await);
    let missing = acl(&state, "/no-such-acls", vec![]).await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchBucket"),
        "{}",
        missing.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_acl_reads_the_owner_of_an_object_that_exists() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/oacl", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/oacl/k", vec![], b"held").await.status,
        200
    );
    assert_owner_full_control(&acl(&state, "/oacl/k", vec![]).await);
    let seen = put_acl(
        &state,
        "/oacl/k",
        vec![("x-amz-acl", "bucket-owner-full-control")],
        b"",
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    let seen = put_acl(&state, "/oacl/k", vec![("x-amz-acl", "public-read")], b"").await;
    assert_eq!(
        seen.code.as_deref(),
        Some("AccessControlListNotSupported"),
        "{}",
        seen.body
    );
    let missing = acl(&state, "/oacl/absent", vec![]).await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchKey"),
        "{}",
        missing.body
    );
    let missing = put_acl(&state, "/oacl/absent", vec![("x-amz-acl", "private")], b"").await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchKey"),
        "{}",
        missing.body
    );
    let versioning =
        br#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#;
    assert_eq!(
        call(
            &state,
            "PUT",
            "/oacl",
            vec![("versioning", None)],
            versioning
        )
        .await
        .status,
        200
    );
    let written = call(&state, "PUT", "/oacl/v", vec![], b"one").await;
    let version = written.headers["x-amz-version-id"]
        .to_str()
        .expect("ascii")
        .to_owned();
    assert_owner_full_control(&acl(&state, "/oacl/v", vec![("versionId", Some(&version))]).await);
    let other = acl(
        &state,
        "/oacl/v",
        vec![("versionId", Some("0000000000000000000000000000000a"))],
    )
    .await;
    assert!(
        matches!(
            other.code.as_deref(),
            Some("NoSuchVersion" | "InvalidArgument")
        ),
        "{}",
        other.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn ownership_controls_are_always_bucket_owner_enforced() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/owned", vec![], b"").await.status, 200);
    let rule = |owner: &str| {
        format!(
            r#"<OwnershipControls xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Rule><ObjectOwnership>{owner}</ObjectOwnership></Rule></OwnershipControls>"#
        )
    };
    let read = call(
        &state,
        "GET",
        "/owned",
        vec![("ownershipControls", None)],
        b"",
    )
    .await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(
        read.body
            .contains("<Rule><ObjectOwnership>BucketOwnerEnforced</ObjectOwnership></Rule>"),
        "{}",
        read.body
    );
    let enforced = rule("BucketOwnerEnforced");
    let seen = call(
        &state,
        "PUT",
        "/owned",
        vec![("ownershipControls", None)],
        enforced.as_bytes(),
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    for other in ["ObjectWriter", "BucketOwnerPreferred"] {
        let body = rule(other);
        let seen = call(
            &state,
            "PUT",
            "/owned",
            vec![("ownershipControls", None)],
            body.as_bytes(),
        )
        .await;
        assert_eq!(seen.status, 501, "{other}: {}", seen.body);
        assert_eq!(seen.code.as_deref(), Some("NotImplemented"));
    }
    let seen = call(
        &state,
        "PUT",
        "/owned",
        vec![("ownershipControls", None)],
        b"<Nope/>",
    )
    .await;
    assert_eq!(seen.code.as_deref(), Some("MalformedXML"), "{}", seen.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_of_another_space_reads_no_acl() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/aclkept", vec![], b"").await.status,
        200
    );
    let key = user_with_key(state.storage(), "stranger", "elsewhere", Role::Member, true).await;
    for query in [vec![("acl", None)], vec![("ownershipControls", None)]] {
        let seen = call_as(&state, key.pair(), "GET", "/aclkept", query, vec![], b"").await;
        assert_eq!(seen.code.as_deref(), Some("AccessDenied"), "{}", seen.body);
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_reader_reads_the_acls_and_writes_none() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "aclspace", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "aclspace", Role::Member, false).await;
    let as_ann = |method, path, query, body: &'static [u8]| {
        call_as(&state, ann.pair(), method, path, query, vec![], body)
    };
    assert_eq!(as_ann("PUT", "/ann-acl", vec![], b"").await.status, 200);
    assert_eq!(
        as_ann("PUT", "/ann-acl/k", vec![], b"held").await.status,
        200
    );
    grant(
        storage,
        "bob",
        "ann-acl",
        Access {
            read: true,
            ..Access::default()
        },
    )
    .await;
    for path in ["/ann-acl", "/ann-acl/k"] {
        let read = call_as(
            &state,
            bob.pair(),
            "GET",
            path,
            vec![("acl", None)],
            vec![],
            b"",
        )
        .await;
        assert_eq!(read.status, 200, "{path}: {}", read.body);
        let write = call_as(
            &state,
            bob.pair(),
            "PUT",
            path,
            vec![("acl", None)],
            vec![("x-amz-acl", "private")],
            b"",
        )
        .await;
        assert_eq!(
            write.code.as_deref(),
            Some("AccessDenied"),
            "{path}: {}",
            write.body
        );
    }
    let enforced = br#"<OwnershipControls xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Rule><ObjectOwnership>BucketOwnerEnforced</ObjectOwnership></Rule></OwnershipControls>"#;
    let write = call_as(
        &state,
        bob.pair(),
        "PUT",
        "/ann-acl",
        vec![("ownershipControls", None)],
        vec![],
        enforced,
    )
    .await;
    assert_eq!(
        write.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        write.body
    );
}
