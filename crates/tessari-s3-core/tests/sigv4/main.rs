//! SigV4 against AWS's published examples, then each example mutated in one place and refused.
//!
//! Credentials, date and region are the ones AWS publishes for its S3 SigV4 examples. The presigned GET, the
//! streaming seed and the two data-chunk signatures are the published values. The other expected signatures are
//! AWS's header-auth and final-chunk examples: a 256-bit HMAC cannot be matched by an implementation that rebuilt a
//! different request, so reproducing one verifies the request it was computed over as well as the code.

#![cfg(test)]

mod header;
mod presign;
mod streaming;

use tessari_s3_core::auth::{
    AuthError, AuthorizationHeader, SecretKey, SignedRequest, Verified, verify_header,
};

pub(crate) const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
pub(crate) const REGION: &str = "us-east-1";
/// 2013-05-24T00:00:00Z, the published examples' signing time.
pub(crate) const NOW: i64 = 1_369_353_600;
pub(crate) const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
pub(crate) const CREDENTIAL: &str =
    "Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request";

pub(crate) fn secret() -> SecretKey {
    SecretKey::new(SECRET.to_owned())
}

pub(crate) fn authorization(signed_headers: &str, signature: &str) -> String {
    format!("AWS4-HMAC-SHA256 {CREDENTIAL},SignedHeaders={signed_headers},Signature={signature}")
}

pub(crate) fn verify(
    request: &SignedRequest<'_>,
    region: &str,
    now: i64,
) -> Result<Verified, AuthError> {
    let value = request
        .header("authorization")
        .ok_or(AuthError::Malformed {
            reason: "test: no header",
        })?;
    let auth = AuthorizationHeader::parse(value)?;
    assert_eq!(auth.access_key(), "AKIAIOSFODNN7EXAMPLE");
    verify_header(request, &auth, &secret(), region, now)
}

pub(crate) fn check(
    method: &str,
    path: &str,
    query: &str,
    owned: &[(&str, String)],
    region: &str,
    now: i64,
) -> Result<Verified, AuthError> {
    let headers: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let request = SignedRequest {
        method,
        raw_path: path,
        raw_query: query,
        headers: &headers,
    };
    verify(&request, region, now)
}
