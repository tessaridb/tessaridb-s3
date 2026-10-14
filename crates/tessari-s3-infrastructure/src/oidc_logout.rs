//! Believing a back-channel logout token (OpenID Connect Back-Channel Logout 1.0) only as far as an ID token would be
//! believed: the algorithm from the console's own list, never `none` or an HMAC; the key by its id from the
//! provider's published set, fetched again once for an id the console does not hold; then our issuer, our client,
//! issued within the last five minutes, the logout event, a subject, no nonce, and an id the route keeps so it is
//! taken once. The library has no logout-token type, so the compact token is read here and only the signature check
//! is the library's.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openidconnect::core::{CoreJsonWebKeyUse, CoreJwsSigningAlgorithm, CoreProviderMetadata};
use openidconnect::{JsonWebKey, JsonWebKeyId};
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use tessari_s3_constants::{CONSOLE_OIDC_LOGOUT_AHEAD_SECS, CONSOLE_OIDC_LOGOUT_MAX_AGE_SECS};

use crate::oidc::OidcClient;
use crate::oidc_verify::{ALLOWED_ALGORITHMS, OidcError};

const LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";

/// Who the provider says signed out, and the token's id, which may be acted on once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedOut {
    /// The configured issuer.
    pub issuer: String,
    /// The provider's subject for the person.
    pub subject: String,
    /// The token's `jti`.
    pub jti: String,
}

#[derive(Deserialize)]
struct Header {
    alg: CoreJwsSigningAlgorithm,
    kid: Option<JsonWebKeyId>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    iat: i64,
    jti: Option<String>,
    sub: Option<String>,
    events: Option<HashMap<String, IgnoredAny>>,
    nonce: Option<IgnoredAny>,
}

fn refused(reason: &'static str) -> OidcError {
    OidcError::LogoutRefused(reason)
}

fn part<T: DeserializeOwned>(text: &str) -> Result<T, OidcError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| refused("malformed"))?;
    serde_json::from_slice(&bytes).map_err(|_| refused("malformed"))
}

/// Whether `metadata` holds a signing key named `kid` (any, when the token names none) that signed `input`.
fn signed_by(
    metadata: &CoreProviderMetadata,
    header: &Header,
    input: &[u8],
    signature: &[u8],
) -> bool {
    metadata.jwks().keys().iter().any(|key| {
        let named = header
            .kid
            .as_ref()
            .is_none_or(|kid| key.key_id() == Some(kid));
        let for_signing = key
            .key_use()
            .is_none_or(|key_use| matches!(key_use, CoreJsonWebKeyUse::Signature));
        named && for_signing && key.verify_signature(&header.alg, input, signature).is_ok()
    })
}

impl OidcClient {
    /// The person a back-channel logout `token` signs out.
    ///
    /// # Errors
    /// [`OidcError::LogoutRefused`] naming the first check that failed; [`OidcError::Unavailable`] when the provider's
    /// keys cannot be had.
    pub async fn verify_logout(&self, token: &str) -> Result<LoggedOut, OidcError> {
        let mut parts = token.split('.');
        let (Some(head), Some(body), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(refused("malformed"));
        };
        let header: Header = part(head)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| refused("malformed"))?;
        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(refused("algorithm"));
        }
        let input = format!("{head}.{body}");
        let held = self.discovered(false).await?;
        let signed = signed_by(&held, &header, input.as_bytes(), &signature) || {
            // A key the console does not hold: the provider may have rotated; ask once, then decide.
            let refreshed = self.discovered(true).await?;
            signed_by(&refreshed, &header, input.as_bytes(), &signature)
        };
        if !signed {
            return Err(refused("signature"));
        }
        let claims: Claims = part(body)?;
        let settings = self.settings();
        if claims.iss != settings.issuer {
            return Err(refused("issuer"));
        }
        let addressed = match &claims.aud {
            Audience::One(audience) => *audience == settings.client_id,
            Audience::Many(audiences) => audiences.contains(&settings.client_id),
        };
        if !addressed {
            return Err(refused("audience"));
        }
        let now = crate::oidc::now_secs();
        let age = now.saturating_sub(claims.iat);
        if !(-CONSOLE_OIDC_LOGOUT_AHEAD_SECS..=CONSOLE_OIDC_LOGOUT_MAX_AGE_SECS).contains(&age) {
            return Err(refused("issued at"));
        }
        if !claims
            .events
            .is_some_and(|events| events.contains_key(LOGOUT_EVENT))
        {
            return Err(refused("event"));
        }
        if claims.nonce.is_some() {
            return Err(refused("nonce"));
        }
        let subject = claims.sub.ok_or_else(|| refused("subject"))?;
        let jti = claims
            .jti
            .filter(|jti| !jti.is_empty())
            .ok_or_else(|| refused("jti"))?;
        Ok(LoggedOut {
            issuer: settings.issuer.clone(),
            subject,
            jti,
        })
    }
}
