//! Presigned URLs.

use tessari_s3_core::auth::{
    AuthError, PayloadHash, PresignedQuery, SignedRequest, verify_presigned,
};

use crate::{NOW, REGION, secret};

// ---- presigned GET ----

const PRESIGN_SIGNATURE: &str = "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404";

fn presign_query(expires: &str, signature: &str) -> String {
    format!(
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
         &X-Amz-Date=20130524T000000Z&X-Amz-Expires={expires}&X-Amz-SignedHeaders=host&X-Amz-Signature={signature}"
    )
}

fn check_presigned(query: &str, now: i64) -> Result<tessari_s3_core::auth::Verified, AuthError> {
    let headers = [("Host", "examplebucket.s3.amazonaws.com")];
    let request = SignedRequest {
        method: "GET",
        raw_path: "/test.txt",
        raw_query: query,
        headers: &headers,
    };
    let parsed = PresignedQuery::parse(query)?.ok_or(AuthError::Malformed {
        reason: "test: no presign",
    })?;
    assert_eq!(parsed.access_key(), "AKIAIOSFODNN7EXAMPLE");
    verify_presigned(&request, &parsed, &secret(), REGION, now)
}

#[test]
fn presigned_get_reproduces_the_published_signature() {
    let verified = check_presigned(&presign_query("86400", PRESIGN_SIGNATURE), NOW + 3600)
        .expect("published example verifies");
    assert_eq!(verified.payload(), PayloadHash::Unsigned);
}

#[test]
fn presigned_get_is_refused_after_it_expires_and_before_it_is_valid() {
    let query = presign_query("86400", PRESIGN_SIGNATURE);
    assert!(
        check_presigned(&query, NOW + 86_400).is_ok(),
        "valid through its last second"
    );
    assert_eq!(
        check_presigned(&query, NOW + 86_401).map(|_| ()),
        Err(AuthError::PresignExpired)
    );
    assert_eq!(
        check_presigned(&query, NOW - 901).map(|_| ()),
        Err(AuthError::PresignExpired)
    );
}

#[test]
fn presign_lifetime_outside_one_second_to_seven_days_is_refused() {
    for expires in ["0", "604801", "-1", "x"] {
        let refusal = check_presigned(&presign_query(expires, PRESIGN_SIGNATURE), NOW).map(|_| ());
        assert_eq!(
            refusal,
            Err(AuthError::PresignExpiresOutOfRange),
            "{expires}"
        );
    }
}

#[test]
fn presigned_get_with_a_changed_lifetime_is_refused() {
    let refusal = check_presigned(&presign_query("86401", PRESIGN_SIGNATURE), NOW).map(|_| ());
    assert_eq!(refusal, Err(AuthError::SignatureMismatch));
}

#[test]
fn a_query_without_presign_parameters_is_not_a_presign() {
    assert_eq!(PresignedQuery::parse("list-type=2&prefix=a"), Ok(None));
}
