//! The aws-chunked signature chain: every chunk signed over the previous signature, starting from the seed.

use sha2::{Digest, Sha256};
use tessari_s3_constants::STREAMING_MIN_CHUNK_LEN;

use super::encode::hex_lower;
use super::signing::check_signature;
use super::verified::Verified;
use super::{AuthError, AuthResult};

/// SHA-256 of the empty string, a fixed line of every chunk's string to sign.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Verifies the chunks of one streaming body in order, so a chunk swapped, dropped, appended or altered is refused
/// before its bytes are handed on.
#[derive(Debug)]
pub struct ChunkChain {
    verified: Verified,
    decoded_len: u64,
    received: u64,
    short_chunk_seen: bool,
    finished: bool,
}

impl ChunkChain {
    /// A chain seeded with the request's own signature, for a body of `decoded_len` bytes
    /// (`x-amz-decoded-content-length`).
    #[must_use]
    pub const fn new(verified: Verified, decoded_len: u64) -> Self {
        Self {
            verified,
            decoded_len,
            received: 0,
            short_chunk_seen: false,
            finished: false,
        }
    }

    /// Verifies the next chunk; an empty `data` is the final chunk.
    ///
    /// # Errors
    /// [`AuthError::SignatureMismatch`], [`AuthError::MalformedChunk`] or [`AuthError::DecodedLengthMismatch`].
    pub fn verify_chunk(&mut self, data: &[u8], signature_hex: &str) -> AuthResult<()> {
        if self.finished {
            return Err(AuthError::MalformedChunk {
                reason: "data after the final chunk",
            });
        }
        if self.short_chunk_seen && !data.is_empty() {
            return Err(AuthError::MalformedChunk {
                reason: "a chunk shorter than 8 KiB before the last",
            });
        }
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256-PAYLOAD\n{}\n{}\n{}\n{EMPTY_SHA256}\n{}",
            self.verified.datetime.as_str(),
            self.verified.scope.to_line(),
            hex_lower(&self.verified.signature),
            hex_lower(&Sha256::digest(data)),
        );
        let computed = self.verified.signing_key.sign(string_to_sign.as_bytes())?;
        check_signature(&computed, signature_hex)?;
        self.verified.signature = computed;
        let len = u64::try_from(data.len()).map_err(|_| AuthError::DecodedLengthMismatch)?;
        self.received = self
            .received
            .checked_add(len)
            .ok_or(AuthError::DecodedLengthMismatch)?;
        if self.received > self.decoded_len {
            return Err(AuthError::DecodedLengthMismatch);
        }
        if data.is_empty() {
            self.finished = true;
            if self.received != self.decoded_len {
                return Err(AuthError::DecodedLengthMismatch);
            }
        } else if len < STREAMING_MIN_CHUNK_LEN {
            self.short_chunk_seen = true;
        }
        Ok(())
    }

    /// Ends the body; the final zero-length chunk must have been verified.
    ///
    /// # Errors
    /// [`AuthError::MalformedChunk`] when the stream ended early.
    pub fn finish(&self) -> AuthResult<()> {
        if self.finished {
            Ok(())
        } else {
            Err(AuthError::MalformedChunk {
                reason: "the body ended before the final chunk",
            })
        }
    }
}

/// Parses a chunk header line `hex-size;chunk-signature=<64 hex>` (without the CRLF).
///
/// # Errors
/// [`AuthError::MalformedChunk`].
pub fn parse_chunk_header(line: &str) -> AuthResult<(u64, &str)> {
    let malformed = AuthError::MalformedChunk {
        reason: "chunk header",
    };
    let (size_hex, extension) = line.split_once(';').ok_or(malformed.clone())?;
    let signature = extension
        .strip_prefix("chunk-signature=")
        .ok_or(malformed.clone())?;
    if size_hex.is_empty() || !super::signing::is_signature_hex(signature) {
        return Err(malformed);
    }
    let size = u64::from_str_radix(size_hex, 16).map_err(|_| malformed)?;
    Ok((size, signature))
}

#[cfg(test)]
mod tests {
    use super::{ChunkChain, EMPTY_SHA256};
    use crate::auth::AuthError;
    use crate::auth::encode::hex_lower;
    use crate::auth::signing::{CredentialScope, SecretKey, SigningKey};
    use crate::auth::time::AmzDateTime;
    use crate::auth::verified::{PayloadHash, Verified};
    use sha2::{Digest, Sha256};

    fn seeded() -> Verified {
        let scope =
            CredentialScope::parse("20130524", "us-east-1", "s3", "aws4_request").expect("scope");
        let signing_key =
            SigningKey::derive(&SecretKey::new("secret".to_owned()), &scope).expect("key");
        Verified {
            access_key: "AK".to_owned(),
            scope,
            datetime: AmzDateTime::parse("20130524T000000Z").expect("date"),
            signing_key,
            signature: [7; 32],
            payload: PayloadHash::StreamingSigned,
        }
    }

    /// Signs `data` as the chain's next chunk would be signed by an honest client.
    fn sign_next(chain: &ChunkChain, data: &[u8]) -> String {
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256-PAYLOAD\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n{}\n{EMPTY_SHA256}\n{}",
            hex_lower(&chain.verified.signature),
            hex_lower(&Sha256::digest(data))
        );
        hex_lower(
            &chain
                .verified
                .signing_key
                .sign(string_to_sign.as_bytes())
                .expect("sign"),
        )
    }

    #[test]
    fn a_short_chunk_may_only_be_the_last_data_chunk() {
        let mut chain = ChunkChain::new(seeded(), 2_048);
        let short = [1_u8; 1_024];
        let signature = sign_next(&chain, &short);
        assert_eq!(chain.verify_chunk(&short, &signature), Ok(()));
        let signature = sign_next(&chain, &short);
        let refusal = chain.verify_chunk(&short, &signature);
        assert_eq!(
            refusal,
            Err(AuthError::MalformedChunk {
                reason: "a chunk shorter than 8 KiB before the last"
            })
        );
    }

    #[test]
    fn a_short_last_chunk_then_the_final_chunk_completes() {
        let mut chain = ChunkChain::new(seeded(), 8_192 + 10);
        for data in [&[2_u8; 8_192][..], &[3_u8; 10][..], &[][..]] {
            let signature = sign_next(&chain, data);
            assert_eq!(chain.verify_chunk(data, &signature), Ok(()));
        }
        assert_eq!(chain.finish(), Ok(()));
        let signature = sign_next(&chain, &[]);
        let refusal = chain.verify_chunk(&[], &signature);
        assert_eq!(
            refusal,
            Err(AuthError::MalformedChunk {
                reason: "data after the final chunk"
            })
        );
    }
}
