//! A bucket policy binds every path (S3 ref 04 M1, M6, M7): an anonymous caller reaches only what a public statement
//! grants, an explicit Deny outweighs the space's administrator, each key of a batch delete and a copy's source are
//! asked on their own, and a key's absence is told only to a caller who may list.

use axum::body::Body;
use axum::http::Request;
use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::delete_many::md5_of;
use crate::policy::{public_read, put_policy};
use crate::{IGNORED, Seen, call, call_as, call_tls, call_with, fresh, send};

/// A request with no credentials at all.
pub(crate) async fn anonymous(
    state: &tessari_s3_api::ApiState,
    method: &str,
    path: &str,
    body: &[u8],
) -> Seen {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost:9100")
        .header("content-length", body.len().to_string())
        .body(Body::from(body.to_vec()))
        .expect("request");
    send(state, request).await
}

fn policy(statements: &str) -> String {
    format!(r#"{{"Version":"2012-10-17","Statement":[{statements}]}}"#)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_anonymous_caller_reaches_only_what_a_public_statement_grants() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/pub", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/pub/a.txt", vec![], b"hello")
            .await
            .status,
        200
    );
    let before = anonymous(&state, "GET", "/pub/a.txt", b"").await;
    assert_eq!(
        (before.status, before.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    crate::public_access::allow_public(&state, "pub").await;
    assert_eq!(
        put_policy(&state, "pub", &public_read("pub")).await.status,
        204
    );
    let read = anonymous(&state, "GET", "/pub/a.txt", b"").await;
    assert_eq!((read.status, read.body.as_str()), (200, "hello"));
    let write = anonymous(&state, "PUT", "/pub/b.txt", b"x").await;
    assert_eq!(
        write.code.as_deref(),
        Some("AccessDenied"),
        "a read grant is not a write grant"
    );
    let listing = anonymous(&state, "GET", "/pub", b"").await;
    assert_eq!(listing.code.as_deref(), Some("AccessDenied"));
    // Without s3:ListBucket a missing key is not told apart from a forbidden one.
    let missing = anonymous(&state, "GET", "/pub/none.txt", b"").await;
    assert_eq!(
        (missing.status, missing.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    let with_list = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":["s3:GetObject","s3:ListBucket"],"Resource":["arn:aws:s3:::pub","arn:aws:s3:::pub/*"]}"#,
    );
    assert_eq!(put_policy(&state, "pub", &with_list).await.status, 204);
    let told = anonymous(&state, "GET", "/pub/none.txt", b"").await;
    assert_eq!(
        (told.status, told.code.as_deref()),
        (404, Some("NoSuchKey"))
    );
    assert_eq!(anonymous(&state, "GET", "/pub", b"").await.status, 200);
    assert_eq!(
        anonymous(&state, "GET", "/", b"").await.code.as_deref(),
        Some("AccessDenied")
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_explicit_deny_outweighs_the_space_administrator_on_every_path() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let admin = user_with_key(storage, "boss", "default", Role::SpaceAdmin, true).await;
    let member = user_with_key(storage, "ann", "default", Role::Member, false).await;
    assert_eq!(call(&state, "PUT", "/vault", vec![], b"").await.status, 200);
    grant(
        storage,
        "ann",
        "vault",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    for key in [
        "/vault/secret/x",
        "/vault/open/x",
        "/vault/keep/a",
        "/vault/tmp/b",
    ] {
        assert_eq!(call(&state, "PUT", key, vec![], b"data").await.status, 200);
    }
    let deny = policy(
        r#"{"Sid":"no-secret","Effect":"Deny","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::vault/secret/*"},
           {"Sid":"keep","Effect":"Deny","Principal":"*","Action":"s3:DeleteObject","Resource":"arn:aws:s3:::vault/keep/*"}"#,
    );
    assert_eq!(put_policy(&state, "vault", &deny).await.status, 204);
    let secret = call_as(
        &state,
        admin.pair(),
        "GET",
        "/vault/secret/x",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(secret.code.as_deref(), Some("AccessDenied"));
    let open = call_as(
        &state,
        admin.pair(),
        "GET",
        "/vault/open/x",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(open.status, 200);
    // Root is the documented exception: it can always reach the bucket to repair its policy.
    assert_eq!(
        call(&state, "GET", "/vault/secret/x", vec![], b"")
            .await
            .status,
        200
    );
    // A copy asks for its source as a read of the source key.
    let copied = call_as(
        &state,
        member.pair(),
        "PUT",
        "/vault/open/copy",
        vec![],
        vec![("x-amz-copy-source", "/vault/secret/x")],
        b"",
    )
    .await;
    assert_eq!(
        copied.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        copied.body
    );
    // Each key of a batch delete is asked on its own and answered on its own.
    let batch =
        "<Delete><Object><Key>keep/a</Key></Object><Object><Key>tmp/b</Key></Object></Delete>";
    let md5 = md5_of(batch);
    let deleted = call_as(
        &state,
        member.pair(),
        "POST",
        "/vault",
        vec![("delete", None)],
        vec![("content-md5", md5.as_str())],
        batch.as_bytes(),
    )
    .await;
    assert_eq!(deleted.status, 200, "{}", deleted.body);
    assert!(
        deleted
            .body
            .contains("<Error><Key>keep/a</Key><Code>AccessDenied</Code>"),
        "{}",
        deleted.body
    );
    assert!(
        deleted.body.contains("<Deleted><Key>tmp/b</Key>"),
        "{}",
        deleted.body
    );
    assert_eq!(
        call(&state, "HEAD", "/vault/keep/a", vec![], b"")
            .await
            .status,
        200
    );
    assert_eq!(
        call(&state, "HEAD", "/vault/tmp/b", vec![], b"")
            .await
            .status,
        404
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_deny_in_the_clear_refuses_plain_http_and_spares_tls() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let admin = user_with_key(storage, "boss", "default", Role::SpaceAdmin, true).await;
    assert_eq!(
        call(&state, "PUT", "/tls-only", vec![], b"").await.status,
        200
    );
    assert_eq!(
        call(&state, "PUT", "/tls-only/k", vec![], b"v")
            .await
            .status,
        200
    );
    let deny = policy(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:*","Resource":["arn:aws:s3:::tls-only","arn:aws:s3:::tls-only/*"],"Condition":{"Bool":{"aws:SecureTransport":"false"}}}"#,
    );
    assert_eq!(put_policy(&state, "tls-only", &deny).await.status, 204);
    let clear = call_as(
        &state,
        admin.pair(),
        "GET",
        "/tls-only/k",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(clear.code.as_deref(), Some("AccessDenied"));
    let _ = call_with;
    let secure = call_tls(&state, "GET", "/tls-only/k", vec![], vec![], b"").await;
    assert_eq!(secure.status, 200, "root over TLS");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_deny_binds_the_console_as_it_binds_the_s3_router() {
    let _why = IGNORED;
    let (state, console) = crate::fresh_console().await;
    let storage = state.storage();
    let admin = user_with_key(storage, "boss", "default", Role::SpaceAdmin, true).await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    for key in ["/held/secret/x", "/held/open/x"] {
        assert_eq!(call(&state, "PUT", key, vec![], b"data").await.status, 200);
    }
    let deny = policy(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::held/secret/*"}"#,
    );
    assert_eq!(put_policy(&state, "held", &deny).await.status, 204);
    let token = crate::console_users::sign_in_as(&console, &admin)
        .await
        .expect("signed in");
    let detail = |key: &'static str| {
        let (console, token) = (console.clone(), token.clone());
        async move {
            crate::console::send(
                &console,
                "GET",
                &format!("/api/v1/buckets/held/object?key={key}"),
                Some(&token),
                None,
            )
            .await
            .status
        }
    };
    assert_eq!(
        detail("open%2Fx").await,
        200,
        "the control: the admin reads what is not denied"
    );
    assert_eq!(detail("secret%2Fx").await, 403);
    let download = crate::console::send(
        &console,
        "GET",
        "/api/v1/buckets/held/object/content?key=secret%2Fx",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(download.status, 403);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn listing_versions_is_its_own_action_a_policy_can_deny() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "default", Role::Member, false).await;
    assert_eq!(
        call(&state, "PUT", "/history", vec![], b"").await.status,
        200
    );
    grant(
        storage,
        "ann",
        "history",
        Access {
            read: true,
            write: false,
        },
    )
    .await;
    let deny = policy(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:ListBucketVersions","Resource":"arn:aws:s3:::history"}"#,
    );
    assert_eq!(put_policy(&state, "history", &deny).await.status, 204);
    let listed = call_as(&state, ann.pair(), "GET", "/history", vec![], vec![], b"").await;
    assert_eq!(
        listed.status, 200,
        "s3:ListBucket is not denied: {}",
        listed.body
    );
    let versions = call_as(
        &state,
        ann.pair(),
        "GET",
        "/history",
        vec![("versions", None)],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (versions.status, versions.code.as_deref()),
        (403, Some("AccessDenied")),
        "ListObjectVersions asks s3:ListBucketVersions: {}",
        versions.body
    );
}
