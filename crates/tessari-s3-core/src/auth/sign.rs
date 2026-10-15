//! SigV4 on the client side: the `Authorization` header for a request, built from the same canonical request,
//! string to sign and key derivation the verifier uses, so there is one SigV4 implementation.

use tessari_s3_types::{SecretKey, Timestamp};

use super::canonical::{SignedRequest, canonical_request};
use super::encode::hex_lower;
use super::signing::{ALGORITHM, CredentialScope, SigningKey, string_to_sign};
use super::time::AmzDateTime;
use super::{AuthError, AuthResult};

/// The `Authorization` header value for `request`, signing every header it carries. The request must carry
/// `x-amz-date` (the signing time) and `x-amz-content-sha256` (the payload hash, signed as written).
///
/// # Errors
/// [`AuthError::InvalidDate`] without a well-formed `x-amz-date`; [`AuthError::MissingContentSha256`] without a
/// payload hash; [`AuthError::Malformed`] for an empty region; canonicalisation errors for a broken escape.
pub fn sign_header(
    request: &SignedRequest<'_>,
    access_key: &str,
    secret: &SecretKey,
    region: &str,
) -> AuthResult<String> {
    let datetime = AmzDateTime::parse(request.header("x-amz-date").ok_or(AuthError::InvalidDate)?)?;
    let payload_hash = request
        .header("x-amz-content-sha256")
        .ok_or(AuthError::MissingContentSha256)?;
    let scope = CredentialScope::parse(datetime.date(), region, "s3", "aws4_request")?;
    let mut signed_headers: Vec<String> = request
        .headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    signed_headers.sort_unstable();
    signed_headers.dedup();
    let canonical = canonical_request(request, None, &signed_headers, payload_hash)?;
    let key = SigningKey::derive(secret, &scope)?;
    let signature = key.sign(string_to_sign(&datetime, &scope, &canonical).as_bytes())?;
    Ok(format!(
        "{ALGORITHM} Credential={access_key}/{},SignedHeaders={},Signature={}",
        scope.to_line(),
        signed_headers.join(";"),
        hex_lower(&signature)
    ))
}

/// The `x-amz-date` text for an instant: `YYYYMMDDTHHMMSSZ`, UTC.
#[must_use]
pub fn amz_date(unix_secs: i64) -> String {
    let instant = Timestamp {
        seconds: unix_secs,
        nanos: 0,
    };
    // `2013-05-24T00:00:00.000Z` without its separators and milliseconds.
    instant
        .iso8601_millis()
        .replace(".000", "")
        .chars()
        .filter(|c| *c != '-' && *c != ':')
        .collect()
}
