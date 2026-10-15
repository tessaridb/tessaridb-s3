//! Continuation tokens for listings: the position a page ended at, opaque and tamper-evident. A token is bound to
//! the bucket, prefix and delimiter it was issued for, so it cannot be replayed into another listing or used to
//! start inside a prefix the request did not name.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tessari_s3_types::SecretKey;
use zeroize::Zeroize;

use super::listing::Resume;

/// The key tokens are authenticated with, derived from the server's root secret so every node sharing that secret
/// accepts every node's tokens.
pub struct TokenKey([u8; 32]);

impl Drop for TokenKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for TokenKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenKey(..)")
    }
}

/// What a token is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenScope<'a> {
    /// The bucket.
    pub bucket: &'a str,
    /// The request's prefix.
    pub prefix: &'a str,
    /// The request's delimiter.
    pub delimiter: Option<&'a str>,
}

/// A token that is not one this server issued for this listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRefused;

impl TokenKey {
    /// The token key for `secret`.
    #[must_use]
    pub fn derive(secret: &SecretKey) -> Self {
        Self(mac(
            secret.expose().as_bytes(),
            &[b"tessaridb-s3 list token"],
        ))
    }
}

/// HMAC-SHA256 over `parts`, each length-prefixed so no two different part lists produce the same input.
fn mac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let Ok(mut hmac) = Hmac::<Sha256>::new_from_slice(key) else {
        // HMAC accepts a key of any length; this branch cannot be taken.
        return [0; 32];
    };
    for part in parts {
        hmac.update(&u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes());
        hmac.update(part);
    }
    hmac.finalize().into_bytes().into()
}

/// The tag and bytes a resume position is carried as.
fn payload(resume: &Resume) -> (u8, &str) {
    match resume {
        Resume::Key(key) => (b'K', key),
        Resume::Prefix(prefix) => (b'P', prefix),
    }
}

fn tag_for(key: &TokenKey, scope: TokenScope<'_>, tag: u8, position: &str) -> [u8; 32] {
    let delimiter: &[u8] = scope.delimiter.map_or(b"\xff-none", str::as_bytes);
    mac(
        &key.0,
        &[
            scope.bucket.as_bytes(),
            scope.prefix.as_bytes(),
            delimiter,
            &[tag],
            position.as_bytes(),
        ],
    )
}

/// The token for resuming at `resume` in the listing `scope`.
#[must_use]
pub fn encode(key: &TokenKey, scope: TokenScope<'_>, resume: &Resume) -> String {
    let (tag, position) = payload(resume);
    let mut raw = Vec::with_capacity(position.len().saturating_add(33));
    raw.extend_from_slice(&tag_for(key, scope, tag, position));
    raw.push(tag);
    raw.extend_from_slice(position.as_bytes());
    URL_SAFE_NO_PAD.encode(raw)
}

/// The position `token` resumes at, when it was issued by this server for exactly this listing.
///
/// # Errors
/// [`TokenRefused`] for anything else.
pub fn decode(key: &TokenKey, scope: TokenScope<'_>, token: &str) -> Result<Resume, TokenRefused> {
    let raw = URL_SAFE_NO_PAD.decode(token).map_err(|_| TokenRefused)?;
    let (mac_part, rest) = raw.split_at_checked(32).ok_or(TokenRefused)?;
    let (&tag, position) = rest.split_first().ok_or(TokenRefused)?;
    let position = std::str::from_utf8(position).map_err(|_| TokenRefused)?;
    let expected = tag_for(key, scope, tag, position);
    if !bool::from(expected.as_slice().ct_eq(mac_part)) {
        return Err(TokenRefused);
    }
    match tag {
        b'K' => Ok(Resume::Key(position.to_owned())),
        b'P' => Ok(Resume::Prefix(position.to_owned())),
        _ => Err(TokenRefused),
    }
}

#[cfg(test)]
mod tests {
    use tessari_s3_types::SecretKey;

    use super::{TokenKey, TokenRefused, TokenScope, decode, encode};
    use crate::objects::listing::Resume;

    fn key(secret: &str) -> TokenKey {
        TokenKey::derive(&SecretKey::new(secret.to_owned()))
    }

    const SCOPE: TokenScope<'static> = TokenScope {
        bucket: "docs",
        prefix: "a/",
        delimiter: Some("/"),
    };

    #[test]
    fn a_token_round_trips_and_is_opaque() {
        let tokens = key("server-secret-0123456789");
        for resume in [
            Resume::Key("a/é €/x".to_owned()),
            Resume::Prefix("a/b/".to_owned()),
        ] {
            let token = encode(&tokens, SCOPE, &resume);
            assert!(
                !token.contains("a/"),
                "the position is not readable: {token}"
            );
            assert!(
                token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "URL-safe: {token}"
            );
            assert_eq!(decode(&tokens, SCOPE, &token), Ok(resume));
        }
    }

    #[test]
    fn a_token_is_refused_anywhere_but_where_it_was_issued() {
        let tokens = key("server-secret-0123456789");
        let token = encode(&tokens, SCOPE, &Resume::Key("a/k".to_owned()));
        let elsewhere = [
            TokenScope {
                bucket: "other",
                ..SCOPE
            },
            TokenScope {
                prefix: "",
                ..SCOPE
            },
            TokenScope {
                delimiter: None,
                ..SCOPE
            },
        ];
        for scope in elsewhere {
            assert_eq!(
                decode(&tokens, scope, &token),
                Err(TokenRefused),
                "{scope:?}"
            );
        }
        assert_eq!(
            decode(&key("another-secret-0123456"), SCOPE, &token),
            Err(TokenRefused),
            "another server's secret"
        );
        let mut tampered = token.clone().into_bytes();
        let last = tampered.len().checked_sub(1).expect("a token is not empty");
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).expect("ascii");
        assert_eq!(decode(&tokens, SCOPE, &tampered), Err(TokenRefused));
        for garbage in ["", "abc", "!!!!", "AAAA"] {
            assert_eq!(
                decode(&tokens, SCOPE, garbage),
                Err(TokenRefused),
                "{garbage}"
            );
        }
    }
}
