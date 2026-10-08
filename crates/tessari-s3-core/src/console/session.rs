//! Console sessions: what an operator holds after signing in. A token names the access key it was issued to and when
//! it stops being accepted, signed with HMAC-SHA256 under the root secret, so any node of the cluster can verify it
//! with no session store and changing the root secret ends every session. It is compared in constant time.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// The version prefix; a new token format gets a new prefix, never a reinterpretation of this one.
const VERSION: &str = "v1";
/// The first line of every message signed, so this signature can never be mistaken for another use of the secret.
const SCHEME: &str = "TESSARI-S3-CONSOLE-SESSION-V1";

/// A verified console session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The access key id the operator signed in with.
    pub key_id: String,
    /// Seconds since the epoch at which the session stops being accepted.
    pub expires: i64,
}

/// Why a console session token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// Not a token of this version.
    #[error("the console session token is malformed")]
    Malformed,
    /// Not signed with this server's key.
    #[error("the console session token is not this server's")]
    Signature,
    /// Past its expiry.
    #[error("the console session has expired")]
    Expired,
}

fn mac(secret: &[u8], payload: &[u8]) -> Vec<u8> {
    // HMAC takes a key of any length, so this cannot fail; an empty answer matches no token (see `verify`).
    Hmac::<Sha256>::new_from_slice(secret)
        .map(|mut mac| {
            mac.update(SCHEME.as_bytes());
            mac.update(b"\n");
            mac.update(payload);
            mac.finalize().into_bytes().to_vec()
        })
        .unwrap_or_default()
}

/// A token for `key_id`, issued at `now` and accepted for `lifetime` seconds; `nonce` makes two tokens of one second
/// differ.
#[must_use]
pub fn issue(secret: &[u8], key_id: &str, now: i64, lifetime: i64, nonce: [u8; 16]) -> String {
    let expires = now.saturating_add(lifetime);
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let payload = format!("{key_id}\n{now}\n{expires}\n{nonce}");
    format!(
        "{VERSION}.{}.{}",
        URL_SAFE_NO_PAD.encode(payload.as_bytes()),
        URL_SAFE_NO_PAD.encode(mac(secret, payload.as_bytes()))
    )
}

/// The session `token` holds, checked against `secret` and the clock `now`.
///
/// # Errors
/// [`SessionError::Malformed`] for anything that is not a token of this version, [`SessionError::Signature`] for one
/// another key signed or anybody changed, [`SessionError::Expired`] at or past its expiry.
pub fn verify(secret: &[u8], token: &str, now: i64) -> Result<Session, SessionError> {
    let mut parts = token.split('.');
    let (Some(VERSION), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(SessionError::Malformed);
    };
    if payload.is_empty() || signature.is_empty() {
        return Err(SessionError::Malformed);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| SessionError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| SessionError::Malformed)?;
    let expected = mac(secret, &payload);
    if expected.is_empty() || !bool::from(expected.ct_eq(&signature)) {
        return Err(SessionError::Signature);
    }
    let text = String::from_utf8(payload).map_err(|_| SessionError::Malformed)?;
    let mut fields = text.split('\n');
    let (Some(key_id), Some(_issued), Some(expires), Some(_nonce), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(SessionError::Malformed);
    };
    let expires = expires
        .parse::<i64>()
        .map_err(|_| SessionError::Malformed)?;
    if now >= expires {
        return Err(SessionError::Expired);
    }
    Ok(Session {
        key_id: key_id.to_owned(),
        expires,
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
