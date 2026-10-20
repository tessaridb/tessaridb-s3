//! Ownership under BucketOwnerEnforced: the owner every answer names is the space — the account a bucket and
//! everything in it belong to — never the credential that asked; a bucket is created only with no ACL or the owner's
//! own, and only with BucketOwnerEnforced.

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::{IGNORED, call, call_as, call_with, fresh};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_owner_named_is_the_space_not_the_key_that_asked() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "owners", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "owners", Role::Member, false).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/owned-by", vec![], vec![], b"")
            .await
            .status,
        200
    );
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/owned-by/k",
            vec![],
            vec![],
            b"x"
        )
        .await
        .status,
        200
    );
    grant(
        storage,
        "bob",
        "owned-by",
        Access {
            read: true,
            ..Access::default()
        },
    )
    .await;
    let owner = "<Owner><ID>owners</ID>";
    for (who, path, query) in [
        (&bob, "/owned-by", vec![("acl", None)]),
        (&bob, "/owned-by/k", vec![("acl", None)]),
        (&ann, "/", vec![]),
        (
            &bob,
            "/owned-by",
            vec![("list-type", Some("2")), ("fetch-owner", Some("true"))],
        ),
        (&bob, "/owned-by", vec![("versions", None)]),
    ] {
        let seen = call_as(&state, who.pair(), "GET", path, query, vec![], b"").await;
        assert_eq!(seen.status, 200, "{path}: {}", seen.body);
        assert!(seen.body.contains(owner), "{path}: {}", seen.body);
        assert!(
            !seen.body.contains(who.pair().0),
            "{path} names no key: {}",
            seen.body
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_is_created_only_with_the_owners_acl_and_enforced_ownership() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    for headers in [
        vec![("x-amz-acl", "public-read")],
        vec![("x-amz-grant-read", "id=someone")],
    ] {
        let seen = call_with(&state, "PUT", "/aclmade", vec![], headers, b"").await;
        assert_eq!(seen.status, 400, "{}", seen.body);
        assert_eq!(
            seen.code.as_deref(),
            Some("InvalidBucketAclWithObjectOwnership")
        );
        assert_eq!(
            call(&state, "HEAD", "/aclmade", vec![], b"").await.status,
            404,
            "not created"
        );
    }
    for owner in ["ObjectWriter", "BucketOwnerPreferred"] {
        let seen = call_with(
            &state,
            "PUT",
            "/aclmade",
            vec![],
            vec![("x-amz-object-ownership", owner)],
            b"",
        )
        .await;
        assert_eq!(
            seen.code.as_deref(),
            Some("NotImplemented"),
            "{owner}: {}",
            seen.body
        );
        assert_eq!(
            call(&state, "HEAD", "/aclmade", vec![], b"").await.status,
            404,
            "not created"
        );
    }
    let seen = call_with(
        &state,
        "PUT",
        "/aclmade",
        vec![],
        vec![
            ("x-amz-acl", "private"),
            ("x-amz-object-ownership", "BucketOwnerEnforced"),
        ],
        b"",
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
}
