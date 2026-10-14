//! Console sessions: what an operator holds after signing in. A token names who it was issued to — the access key
//! that signed in, or the S3 user an identity provider vouched for — when its holder authenticated, and when it stops
//! being accepted, signed with HMAC-SHA256 under the root secret, so any node of the cluster can verify it
//! with no session store and changing the root secret ends every session. It is compared in constant time.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// The version prefix; a new token format gets a new prefix, never a reinterpretation of this one.
const VERSION: &str = "v3";
/// The first line of every message signed, so this signature can never be mistaken for another use of the secret.
const SCHEME: &str = "TESSARI-S3-CONSOLE-SESSION-V3";

/// Who a session was issued to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// The access key id an operator signed in with: the root credential or a user's live key.
    Key(String),
    /// The S3 user an identity provider's sign-in was linked to.
    User(String),
}

impl Subject {
    /// The kind and the name in one string, distinct for a key and a user spelled alike.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Key(id) => format!("key:{id}"),
            Self::User(name) => format!("user:{name}"),
        }
    }

    /// How the action record names who acted: an access key by its id, as it always has; a person as `user:<name>`,
    /// which no generated access key id can spell.
    #[must_use]
    pub fn operator(&self) -> String {
        match self {
            Self::Key(id) => id.clone(),
            Self::User(_) => self.label(),
        }
    }

    fn parse(kind: &str, name: &str) -> Option<Self> {
        match kind {
            "key" => Some(Self::Key(name.to_owned())),
            "user" => Some(Self::User(name.to_owned())),
            _ => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Key(_) => "key",
            Self::User(_) => "user",
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Key(name) | Self::User(name) => name,
        }
    }
}

/// A verified console session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Who signed in.
    pub subject: Subject,
    /// Seconds since the epoch at which the session stops being accepted.
    pub expires: i64,
    /// Seconds since the epoch at which its holder last proved who they are; `None` when that is not known.
    pub authenticated: Option<i64>,
}

impl Session {
    /// Whether its holder authenticated at most `max_age` seconds before `now`; an unknown time never is.
    #[must_use]
    pub fn authenticated_within(&self, now: i64, max_age: i64) -> bool {
        self.authenticated
            .is_some_and(|at| now.saturating_sub(at) <= max_age)
    }
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

/// A token for `subject`, issued at `now` and accepted for `lifetime` seconds, whose holder authenticated at
/// `authenticated` (if known); `nonce` makes two tokens of one second differ.
#[must_use]
pub fn issue(
    secret: &[u8],
    subject: &Subject,
    now: i64,
    lifetime: i64,
    authenticated: Option<i64>,
    nonce: [u8; 16],
) -> String {
    let expires = now.saturating_add(lifetime);
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let authenticated = authenticated.map_or_else(|| "-".to_owned(), |at| at.to_string());
    let payload = format!(
        "{}\n{}\n{now}\n{expires}\n{authenticated}\n{nonce}",
        subject.kind(),
        subject.name()
    );
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
    let (
        Some(kind),
        Some(name),
        Some(_issued),
        Some(expires),
        Some(authenticated),
        Some(_nonce),
        None,
    ) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    )
    else {
        return Err(SessionError::Malformed);
    };
    let subject = Subject::parse(kind, name).ok_or(SessionError::Malformed)?;
    let expires = expires
        .parse::<i64>()
        .map_err(|_| SessionError::Malformed)?;
    let authenticated = match authenticated {
        "-" => None,
        at => Some(at.parse::<i64>().map_err(|_| SessionError::Malformed)?),
    };
    if now >= expires {
        return Err(SessionError::Expired);
    }
    Ok(Session {
        subject,
        expires,
        authenticated,
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
