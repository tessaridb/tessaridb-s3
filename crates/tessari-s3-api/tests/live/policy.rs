//! Bucket policies against a real node (S3 ref 04 M4): written after validation, read back as written, removed, and
//! every malformed or unevaluable document refused before it is stored.

use crate::{IGNORED, Seen, call, fresh};

pub(crate) fn public_read(bucket: &str) -> String {
    format!(
        r#"{{"Version":"2012-10-17","Statement":[{{"Sid":"read","Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{bucket}/*"}}]}}"#
    )
}

pub(crate) async fn put_policy(state: &tessari_s3_api::ApiState, bucket: &str, text: &str) -> Seen {
    call(
        state,
        "PUT",
        &format!("/{bucket}"),
        vec![("policy", None)],
        text.as_bytes(),
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_policy_is_stored_read_back_as_written_and_removed() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    let missing = call(&state, "GET", "/held", vec![("policy", None)], b"").await;
    assert_eq!(
        (missing.status, missing.code.as_deref()),
        (404, Some("NoSuchBucketPolicy"))
    );
    let text = public_read("held");
    let put = put_policy(&state, "held", &text).await;
    assert_eq!(put.status, 204, "{}", put.body);
    let read = call(&state, "GET", "/held", vec![("policy", None)], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert_eq!(read.body, text);
    assert_eq!(
        read.headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let removed = call(&state, "DELETE", "/held", vec![("policy", None)], b"").await;
    assert_eq!(removed.status, 204, "{}", removed.body);
    let gone = call(&state, "GET", "/held", vec![("policy", None)], b"").await;
    assert_eq!(gone.code.as_deref(), Some("NoSuchBucketPolicy"));
    // Removing what is not there is not an error.
    assert_eq!(
        call(&state, "DELETE", "/held", vec![("policy", None)], b"")
            .await
            .status,
        204
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_policy_that_cannot_be_evaluated_as_written_is_refused_and_nothing_is_stored() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/strict", vec![], b"").await.status,
        200
    );
    let unknown_key = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Deny","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::strict/*","Condition":{"StringEquals":{"aws:PrincipalOrgID":"o-1"}}}]}"#;
    let refused = put_policy(&state, "strict", unknown_key).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (400, Some("MalformedPolicy"))
    );
    assert!(
        refused.body.contains("aws:PrincipalOrgID"),
        "{}",
        refused.body
    );
    let other_bucket = public_read("someone-else");
    assert_eq!(
        put_policy(&state, "strict", &other_bucket)
            .await
            .code
            .as_deref(),
        Some("MalformedPolicy")
    );
    let read = call(&state, "GET", "/strict", vec![("policy", None)], b"").await;
    assert_eq!(read.code.as_deref(), Some("NoSuchBucketPolicy"));
    let nowhere = put_policy(&state, "no-such-bucket", &public_read("no-such-bucket")).await;
    assert_eq!(nowhere.code.as_deref(), Some("NoSuchBucket"));
}
