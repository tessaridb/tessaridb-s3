//! `x-amz-content-sha256`: parsing the declaration, and verifying the body against it while it streams.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::verified::PayloadHash;
use super::{AuthError, AuthResult};

/// Parses an `x-amz-content-sha256` value.
///
/// # Errors
/// [`AuthError::Malformed`] when it is neither 64 lowercase hex digits nor a known marker.
pub fn parse_payload_hash(value: &str) -> AuthResult<PayloadHash> {
    match value {
        "UNSIGNED-PAYLOAD" => Ok(PayloadHash::Unsigned),
        "STREAMING-AWS4-HMAC-SHA256-PAYLOAD" => Ok(PayloadHash::StreamingSigned),
        "STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER" => Ok(PayloadHash::StreamingSignedTrailer),
        "STREAMING-UNSIGNED-PAYLOAD-TRAILER" => Ok(PayloadHash::StreamingUnsignedTrailer),
        hex => decode_sha256_hex(hex).map(PayloadHash::Sha256),
    }
}

/// Hashes a body as it streams and compares it with the declared SHA-256 at the end, before anything commits.
#[derive(Debug, Clone)]
pub struct PayloadVerifier {
    expected: [u8; 32],
    hasher: Sha256,
}

impl PayloadVerifier {
    /// A verifier for a body declared to hash to `expected`.
    #[must_use]
    pub fn new(expected: [u8; 32]) -> Self {
        Self {
            expected,
            hasher: Sha256::new(),
        }
    }

    /// Feeds the next piece of the body.
    pub fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    /// Ends the body.
    ///
    /// # Errors
    /// [`AuthError::PayloadHashMismatch`] when the body differs from the declaration.
    pub fn finish(self) -> AuthResult<()> {
        let actual: [u8; 32] = self.hasher.finalize().into();
        verify_payload_digest(&self.expected, &actual)
    }
}

/// Compares a body's SHA-256, computed elsewhere in the same pass as its other digests, with the declared one.
///
/// # Errors
/// [`AuthError::PayloadHashMismatch`] when they differ.
pub fn verify_payload_digest(expected: &[u8; 32], actual: &[u8; 32]) -> AuthResult<()> {
    if bool::from(actual.ct_eq(expected)) {
        Ok(())
    } else {
        Err(AuthError::PayloadHashMismatch)
    }
}

/// 64 lowercase hex digits to 32 bytes.
fn decode_sha256_hex(hex: &str) -> AuthResult<[u8; 32]> {
    let malformed = AuthError::Malformed {
        reason: "x-amz-content-sha256",
    };
    let digits = hex.as_bytes();
    if digits.len() != 64 {
        return Err(malformed);
    }
    let mut out = [0_u8; 32];
    let value = |d: u8| match d {
        b'0'..=b'9' => Some(d.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(d.wrapping_sub(b'a').wrapping_add(10)),
        _ => None,
    };
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, [high, low]) in out.iter_mut().zip(pairs) {
        match (value(*high), value(*low)) {
            (Some(high), Some(low)) => *byte = (high << 4) | low,
            _ => return Err(malformed),
        }
    }
    Ok(out)
}
