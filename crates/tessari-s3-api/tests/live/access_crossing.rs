//! What no grant reaches, against a real node: a bucket of another space (and a name it holds), and a copy's source
//! the caller may not read.

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::{IGNORED, call, call_as, fresh};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn nothing_reaches_into_another_space() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let carl = user_with_key(storage, "carl", "beta", Role::SpaceAdmin, true).await;
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/ann-photos",
            vec![],
            vec![],
            b""
        )
        .await
        .status,
        200
    );
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/ann-photos/a.txt",
            vec![],
            vec![],
            b"a"
        )
        .await
        .status,
        200
    );
    grant(
        storage,
        "carl",
        "ann-photos",
        Access {
            read: true,
            write: true,
        },
    )
    .await;

    for (method, path) in [
        ("GET", "/ann-photos/a.txt"),
        ("PUT", "/ann-photos/b.txt"),
        ("HEAD", "/ann-photos"),
        ("DELETE", "/ann-photos"),
    ] {
        let refused = call_as(&state, carl.pair(), method, path, vec![], vec![], b"").await;
        assert_eq!(refused.status, 403, "{method} {path}");
    }
    let taken = call_as(
        &state,
        carl.pair(),
        "PUT",
        "/ann-photos",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (taken.status, taken.code.as_deref()),
        (409, Some("BucketAlreadyExists"))
    );
    let own = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/ann-photos",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (own.status, own.code.as_deref()),
        (409, Some("BucketAlreadyOwnedByYou"))
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_copy_needs_read_on_its_source() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    assert_eq!(call(&state, "PUT", "/vault", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/vault/secret.txt", vec![], b"root only")
            .await
            .status,
        200
    );
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/ann-photos",
            vec![],
            vec![],
            b""
        )
        .await
        .status,
        200
    );

    let copy = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/ann-photos/stolen.txt",
        vec![],
        vec![("x-amz-copy-source", "/vault/secret.txt")],
        b"",
    )
    .await;
    assert_eq!(
        (copy.status, copy.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    let missing = call(&state, "HEAD", "/ann-photos/stolen.txt", vec![], b"").await;
    assert_eq!(missing.status, 404, "nothing was written");
}
