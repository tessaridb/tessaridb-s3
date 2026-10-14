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
use crate::oidc::OidcError;

/// The signature algorithms an ID token may use; anything else, `none` and every HMAC included, is refused.
pub const ALLOWED_ALGORITHMS: [CoreJwsSigningAlgorithm; 3] = [
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EdDsa,
];

/// The ID token's subject and `amr`, after every check against `metadata`'s keys.
pub(crate) fn verify(
    settings: &OidcSettings,
    id_token: &CoreIdToken,
    metadata: &CoreProviderMetadata,
    nonce: &Nonce,
    access_token: &openidconnect::AccessToken,
) -> std::result::Result<(String, Vec<String>), OidcError> {
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
    Ok((claims.subject().as_str().to_owned(), amr))
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
