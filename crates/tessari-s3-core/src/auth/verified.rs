//! What a successful authentication establishes, and what the body still owes.

use super::signing::{CredentialScope, SigningKey};
use super::time::AmzDateTime;

/// How the body is protected, from `x-amz-content-sha256` (or `UNSIGNED-PAYLOAD` for a presigned URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadHash {
    /// The SHA-256 of the whole body, which must be verified before the write commits.
    Sha256([u8; 32]),
    /// Not signed: integrity rests on TLS and on any `x-amz-checksum-*` the request carries.
    Unsigned,
    /// `STREAMING-AWS4-HMAC-SHA256-PAYLOAD`: an aws-chunked body, each chunk signed in a chain from the seed.
    StreamingSigned,
    /// `STREAMING-AWS4-HMAC-SHA256-PAYLOAD-TRAILER`: signed chunks plus a signed trailing checksum.
    StreamingSignedTrailer,
    /// `STREAMING-UNSIGNED-PAYLOAD-TRAILER`: unsigned chunks plus a trailing checksum.
    StreamingUnsignedTrailer,
}

/// A request whose signature verified.
#[derive(Debug, Clone)]
pub struct Verified {
    pub(crate) access_key: String,
    pub(crate) scope: CredentialScope,
    pub(crate) datetime: AmzDateTime,
    pub(crate) signing_key: SigningKey,
    pub(crate) signature: [u8; 32],
    pub(crate) payload: PayloadHash,
}

impl Verified {
    /// The access key the request was signed with: the principal to authorize.
    #[must_use]
    pub fn access_key(&self) -> &str {
        &self.access_key
    }

    /// How the body must still be verified.
    #[must_use]
    pub const fn payload(&self) -> PayloadHash {
        self.payload
    }
}
