//! Checking an ID token the way the console requires: the algorithm from a fixed list, never from the token or the
//! provider's metadata; the key from the provider's published set; no client secret, so no HMAC can verify; then the
//! issuer, the audience, the expiry, the nonce and — when the token carries it — the `at_hash`.

use openidconnect::core::{
    CoreIdToken, CoreIdTokenVerifier, CoreJwsSigningAlgorithm, CoreProviderMetadata,
};
use openidconnect::{
    AccessTokenHash, ClaimsVerificationError, ClientId, IssuerUrl, Nonce,
    SignatureVerificationError,
};

use crate::console_config::OidcSettings;

/// Why a sign-in through the provider was refused. The console answers every one alike; the reason is for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OidcError {
    /// The provider's discovery document or keys could not be had, and none are held.
    #[error("the identity provider is unavailable")]
    Unavailable,
    /// The authorization response's `iss` names another issuer (a mix-up).
    #[error("the authorization response names another issuer")]
    IssuerMismatch,
    /// The provider refused the code: unknown, spent, expired, or the PKCE verifier does not match.
    #[error("the identity provider refused the code")]
    CodeRefused,
    /// The token response carries no ID token.
    #[error("the token response carries no ID token")]
    NoIdToken,
    /// The ID token failed a check: `signature`, `issuer`, `audience`, `expired`, `nonce` or `claims`.
    #[error("the ID token was refused: {0}")]
    IdTokenRefused(&'static str),
    /// The ID token's `at_hash` is not the access token's.
    #[error("the ID token does not match the access token")]
    AccessTokenMismatch,
    /// The ID token's `amr` names none of the configured second factors.
    #[error("the sign-in carries no second factor")]
    NoSecondFactor,
}

/// The signature algorithms an ID token may use; anything else, `none` and every HMAC included, is refused.
pub const ALLOWED_ALGORITHMS: [CoreJwsSigningAlgorithm; 3] = [
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EdDsa,
];

/// What a verified ID token says about its holder.
pub(crate) struct Verified {
    pub(crate) subject: String,
    pub(crate) amr: Vec<String>,
    /// `auth_time`, in seconds since the epoch, when the token carries it.
    pub(crate) auth_time: Option<i64>,
}

/// What the ID token says about its holder, after every check against `metadata`'s keys.
pub(crate) fn verify(
    settings: &OidcSettings,
    id_token: &CoreIdToken,
    metadata: &CoreProviderMetadata,
    nonce: &Nonce,
    access_token: &openidconnect::AccessToken,
) -> std::result::Result<Verified, OidcError> {
    let issuer = IssuerUrl::new(settings.issuer.clone()).map_err(|_| OidcError::Unavailable)?;
    let verifier = CoreIdTokenVerifier::new_public_client(
        ClientId::new(settings.client_id.clone()),
        issuer,
        metadata.jwks().clone(),
    )
    .set_allowed_algs(ALLOWED_ALGORITHMS);
    let claims = id_token
        .claims(&verifier, nonce)
        .map_err(|error| OidcError::IdTokenRefused(reason(&error)))?;
    if let Some(expected) = claims.access_token_hash() {
        let algorithm = id_token
            .signing_alg()
            .map_err(|_| OidcError::IdTokenRefused("signature"))?;
        let key = id_token
            .signing_key(&verifier)
            .map_err(|_| OidcError::IdTokenRefused("signature"))?;
        let actual = AccessTokenHash::from_token(access_token, algorithm, key)
            .map_err(|_| OidcError::AccessTokenMismatch)?;
        if actual != *expected {
            return Err(OidcError::AccessTokenMismatch);
        }
    }
    let amr = claims
        .auth_method_refs()
        .map(|refs| {
            refs.iter()
                .map(|factor| factor.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    Ok(Verified {
        subject: claims.subject().as_str().to_owned(),
        amr,
        auth_time: claims.auth_time().map(|at| at.timestamp()),
    })
}

/// The check an ID-token refusal names; a key id the console does not hold is told apart, so it can refresh.
fn reason(error: &ClaimsVerificationError) -> &'static str {
    match error {
        ClaimsVerificationError::SignatureVerification(
            SignatureVerificationError::NoMatchingKey,
        ) => "unknown key",
        ClaimsVerificationError::SignatureVerification(_)
        | ClaimsVerificationError::Unsupported(_) => "signature",
        ClaimsVerificationError::InvalidIssuer(_) => "issuer",
        ClaimsVerificationError::InvalidAudience(_) => "audience",
        ClaimsVerificationError::Expired(_) => "expired",
        ClaimsVerificationError::InvalidNonce(_) => "nonce",
        _ => "claims",
    }
}
