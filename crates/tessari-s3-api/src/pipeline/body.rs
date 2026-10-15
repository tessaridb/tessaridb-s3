//! Reading a small request body after authentication, bounded before it is read and checked against the payload
//! hash the request signed before anything uses it.

use axum::body::{Body, Bytes, to_bytes};
use tessari_s3_core::auth::{PayloadHash, PayloadVerifier, Verified};
use tessari_s3_types::ErrorCode;

use crate::Error;

/// Largest control-document body this server reads (1 MiB): configuration XML, never object data.
pub const CONTROL_BODY_LIMIT: usize = 1 << 20;

/// The body, at most `limit` bytes, with its declared SHA-256 verified.
///
/// # Errors
/// `MaxMessageLengthExceeded` past the limit, `BadDigest` when the body differs from its declared hash, and
/// `NotImplemented` for an aws-chunked body on an operation that takes a control document.
pub async fn read_verified(body: Body, verified: &Verified, limit: usize) -> Result<Bytes, Error> {
    let bytes = to_bytes(body, limit).await.map_err(|_| {
        Error::new(
            ErrorCode::MaxMessageLengthExceeded,
            "the request body is longer than this operation accepts",
        )
    })?;
    match verified.payload() {
        PayloadHash::Sha256(expected) => {
            let mut verifier = PayloadVerifier::new(expected);
            verifier.update(&bytes);
            verifier.finish()?;
        }
        PayloadHash::Unsigned => {}
        PayloadHash::StreamingSigned
        | PayloadHash::StreamingSignedTrailer
        | PayloadHash::StreamingUnsignedTrailer => {
            return Err(Error::new(
                ErrorCode::NotImplemented,
                "an aws-chunked body is not accepted for this operation",
            ));
        }
    }
    Ok(bytes)
}
