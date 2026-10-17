//! The session token temporary credentials are presented with: `TSS1` and, in unpadded base64url, the expiry (eight
//! bytes, big-endian seconds) followed by 32 random bytes. Only its SHA-256 digest is stored, so a read of the store
//! yields nothing to present. The expiry is readable from the token alone so that a request whose credential the
//! store has already dropped is told it expired rather than that its key does not exist; for a live credential the
//! stored expiry decides.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const PREFIX: &str = "TSS1";
const EXPIRY_LEN: usize = 8;
const RANDOM_LEN: usize = 32;

/// The token for a credential expiring at `expires` (seconds since the epoch), drawn from `random`.
#[must_use]
pub fn compose(expires: i64, random: &[u8; RANDOM_LEN]) -> String {
    let mut body = [0_u8; EXPIRY_LEN + RANDOM_LEN];
    body[..EXPIRY_LEN].copy_from_slice(&expires.to_be_bytes());
    body[EXPIRY_LEN..].copy_from_slice(random);
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(body))
}

/// What is stored for `token`.
#[must_use]
pub fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// The expiry `token` names, when it is one of this server's tokens.
#[must_use]
pub fn expiry(token: &str) -> Option<i64> {
    let body = URL_SAFE_NO_PAD.decode(token.strip_prefix(PREFIX)?).ok()?;
    if body.len() != EXPIRY_LEN + RANDOM_LEN {
        return None;
    }
    let mut seconds = [0_u8; EXPIRY_LEN];
    seconds.copy_from_slice(body.get(..EXPIRY_LEN)?);
    Some(i64::from_be_bytes(seconds))
}

/// Whether `presented` is the token whose digest is `stored`, compared in constant time.
#[must_use]
pub fn matches(presented: &str, stored: &[u8; 32]) -> bool {
    digest(presented).ct_eq(stored).into()
}
