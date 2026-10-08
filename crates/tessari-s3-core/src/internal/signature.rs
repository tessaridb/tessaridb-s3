//! Internal request signatures: HMAC-SHA256 with the cluster secret over a fixed string naming the method, the path
//! with its query, the request's date and the calling node, checked in constant time within a skew window. The body
//! is not signed — shard bytes are hashed per block as they are written, and transport confidentiality is TLS's.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tessari_s3_constants::INTERNAL_MAX_SKEW_SECS;

/// The first line of every string to sign; a new scheme gets a new line, never a reinterpretation of this one.
const SCHEME: &str = "TESSARI-S3-INTERNAL-V1";

/// Hex digits in a signature: 32 bytes of HMAC-SHA256.
const SIGNATURE_HEX_LEN: usize = 64;

/// Why an internal request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InternalAuthError {
    /// The signature is not the cluster's over this request.
    #[error("the internal request signature does not match")]
    Signature,
    /// The request is dated further from this node's clock than the window allows.
    #[error("the internal request is dated outside the allowed skew")]
    Skew,
}

/// What a signature covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternalRequest<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// The path and query exactly as sent.
    pub path: &'a str,
    /// Seconds since the epoch at the sender.
    pub date: i64,
    /// The sending node's id.
    pub node: &'a str,
}

/// The lowercase hex signature of `request` under `secret`.
#[must_use]
pub fn sign(secret: &[u8], request: &InternalRequest<'_>) -> String {
    let InternalRequest {
        method,
        path,
        date,
        node,
    } = request;
    let to_sign = format!("{SCHEME}\n{method}\n{path}\n{date}\n{node}");
    // HMAC takes a key of any length, so this cannot fail; an empty answer would match nothing (see `verify`).
    Hmac::<Sha256>::new_from_slice(secret)
        .map(|mut mac| {
            mac.update(to_sign.as_bytes());
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        })
        .unwrap_or_default()
}

/// Checks `signature` over `request` at time `now`.
///
/// # Errors
/// [`InternalAuthError::Skew`] or [`InternalAuthError::Signature`].
pub fn verify(
    secret: &[u8],
    request: &InternalRequest<'_>,
    signature: &str,
    now: i64,
) -> Result<(), InternalAuthError> {
    let window = u64::try_from(INTERNAL_MAX_SKEW_SECS).unwrap_or(0);
    if now.abs_diff(request.date) > window {
        return Err(InternalAuthError::Skew);
    }
    let expected = sign(secret, request);
    let matches = expected.len() == SIGNATURE_HEX_LEN
        && bool::from(expected.as_bytes().ct_eq(signature.as_bytes()));
    if matches {
        Ok(())
    } else {
        Err(InternalAuthError::Signature)
    }
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;
