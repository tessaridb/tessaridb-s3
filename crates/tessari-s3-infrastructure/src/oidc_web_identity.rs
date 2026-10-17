//! Believing a web identity token presented for temporary credentials exactly as far as the console believes an ID
//! token: the algorithm from [`ALLOWED_ALGORITHMS`], the key by its id from the provider's published set (fetched
//! again once for an id not held), our issuer, our client among the audiences, not expired, and one of the configured
//! second factors in `amr` — else this would be a way around the console's own sign-in. There is no nonce to check:
//! the token was obtained by the client, not in a flow this server started.

use std::str::FromStr;

use openidconnect::core::{CoreIdToken, CoreIdTokenVerifier, CoreProviderMetadata};
use openidconnect::{ClientId, IssuerUrl, Nonce};

use crate::oidc::OidcClient;
use crate::oidc_verify::{ALLOWED_ALGORITHMS, OidcError, reason};

/// Who a verified web identity token names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebIdentity {
    /// The configured issuer.
    pub issuer: String,
    /// The provider's subject for the person.
    pub subject: String,
    /// The audience the token was addressed to: the configured client id.
    pub audience: String,
}

/// The token's subject and `amr`, after every check against `metadata`'s keys.
fn checked(
    client: &OidcClient,
    token: &CoreIdToken,
    metadata: &CoreProviderMetadata,
) -> Result<(String, Vec<String>), OidcError> {
    let settings = client.settings();
    let issuer = IssuerUrl::new(settings.issuer.clone()).map_err(|_| OidcError::Unavailable)?;
    let verifier = CoreIdTokenVerifier::new_public_client(
        ClientId::new(settings.client_id.clone()),
        issuer,
        metadata.jwks().clone(),
    )
    .set_allowed_algs(ALLOWED_ALGORITHMS);
    let claims = token
        .claims(&verifier, |_: Option<&Nonce>| Ok::<(), String>(()))
        .map_err(|error| OidcError::IdTokenRefused(reason(&error)))?;
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

impl OidcClient {
    /// The person web identity `token` names.
    ///
    /// # Errors
    /// [`OidcError::IdTokenRefused`] naming the check that failed (`malformed` for a token that is not a signed JWT,
    /// `expired` once its `exp` has passed), [`OidcError::NoSecondFactor`] when its `amr` names none of the
    /// configured factors, [`OidcError::Unavailable`] when the provider's keys cannot be had.
    pub async fn verify_web_identity(&self, token: &str) -> Result<WebIdentity, OidcError> {
        let token =
            CoreIdToken::from_str(token).map_err(|_| OidcError::IdTokenRefused("malformed"))?;
        let held = self.discovered(false).await?;
        let (subject, amr) = match checked(self, &token, &held) {
            Err(OidcError::IdTokenRefused("unknown key")) => {
                // A key this server does not hold: the provider may have rotated; ask once, then decide.
                let refreshed = self.discovered(true).await?;
                checked(self, &token, &refreshed)
            }
            other => other,
        }
        .map_err(|error| match error {
            OidcError::IdTokenRefused("unknown key") => OidcError::IdTokenRefused("signature"),
            other => other,
        })?;
        let settings = self.settings();
        if !amr.iter().any(|factor| settings.mfa_amr.contains(factor)) {
            return Err(OidcError::NoSecondFactor);
        }
        Ok(WebIdentity {
            issuer: settings.issuer.clone(),
            subject,
            audience: settings.client_id.clone(),
        })
    }
}
