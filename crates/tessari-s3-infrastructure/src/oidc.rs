//! The console's client of an OpenID Connect provider (the relying-party half; the provider is the operator's): the
//! authorization code flow with PKCE (`S256`), and every check the return leg owes — the `iss` response parameter,
//! the code exchanged once with its verifier, and the ID token's signature, issuer, audience, expiry, nonce,
//! `at_hash` and second factor.
//!
//! The ID token is verified with THIS client's verifier, not the library's default: the algorithms come from
//! [`ALLOWED_ALGORITHMS`], never from the provider's discovery document or the token, and no client secret is handed
//! to it, so no HMAC signature can verify. The provider's discovery document and keys are fetched when first needed
//! (start-up does not depend on the provider), kept as the last known good, and fetched again when a token names a
//! key id the console does not hold — at most once per refresh interval, the claim taken before the fetch.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, TokenResponse,
};
use tessari_s3_types::SecretKey;

use crate::Result;
use crate::console_config::OidcSettings;
use crate::oidc_http::ProviderHttp;
pub use crate::oidc_logout::LoggedOut;
use crate::oidc_verify::verify;
pub use crate::oidc_verify::{ALLOWED_ALGORITHMS, OidcError};

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// A sign-in sent to the provider: where to send the browser, and what its callback must match.
#[derive(Debug, Clone)]
pub struct Started {
    /// The authorization URL.
    pub url: String,
    /// The `state` the callback carries back.
    pub state: String,
    /// The nonce the ID token must carry.
    pub nonce: SecretKey,
    /// The PKCE verifier the code is exchanged with.
    pub verifier: SecretKey,
}

/// Who the provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIdentity {
    /// The configured issuer.
    pub issuer: String,
    /// The provider's subject for the person.
    pub subject: String,
    /// When the person last authenticated at the provider (`auth_time`, never later than now); `None` when the ID
    /// token does not say.
    pub authenticated: Option<i64>,
}

/// The console's OpenID Connect client; cheap to clone, every clone sharing what it fetched.
#[derive(Clone)]
pub struct OidcClient {
    inner: Arc<Inner>,
}

struct Inner {
    settings: OidcSettings,
    http: ProviderHttp,
    refresh_min_secs: i64,
    /// The last discovery document (with its keys) fetched; replaced whole, never held across an `.await`.
    held: Mutex<Option<Arc<CoreProviderMetadata>>>,
    /// When the last fetch was claimed, in seconds since the epoch; 0 before the first.
    last_fetch: AtomicI64,
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(i64::MAX)
}

impl OidcClient {
    /// A client for the provider `settings` names, fetching its keys again at most once per `refresh_min_secs`.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] when the HTTP client cannot be built.
    pub fn new(settings: &OidcSettings, refresh_min_secs: i64) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                settings: settings.clone(),
                http: ProviderHttp::new()?,
                refresh_min_secs,
                held: Mutex::new(None),
                last_fetch: AtomicI64::new(0),
            }),
        })
    }

    /// The request function the library calls: each request's future owns its client, so it borrows nothing.
    fn http(
        &self,
    ) -> impl Fn(
        openidconnect::HttpRequest,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<
                        openidconnect::HttpResponse,
                        crate::oidc_http::ProviderHttpError,
                    >,
                > + Send,
        >,
    > + use<> {
        let http = self.inner.http.clone();
        move |request| {
            let http = http.clone();
            Box::pin(async move { http.send(request).await })
        }
    }

    /// The configured issuer: the one identities are linked under.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.inner.settings.issuer
    }

    /// The provider's settings.
    pub(crate) fn settings(&self) -> &OidcSettings {
        &self.inner.settings
    }

    fn held(&self) -> Option<Arc<CoreProviderMetadata>> {
        self.inner.held.lock().ok().and_then(|held| held.clone())
    }

    /// The provider's discovery document and keys: what is held, unless `refresh` asks for a fetch; a fetch happens
    /// only when this caller claims it and the interval since the last claim has passed, and a failed one keeps
    /// what was held.
    pub(crate) async fn discovered(
        &self,
        refresh: bool,
    ) -> std::result::Result<Arc<CoreProviderMetadata>, OidcError> {
        let held = self.held();
        if let (Some(held), false) = (&held, refresh) {
            return Ok(Arc::clone(held));
        }
        let now = now_secs();
        let last = self.inner.last_fetch.load(Ordering::Acquire);
        let due = last == 0 || now.saturating_sub(last) >= self.inner.refresh_min_secs;
        let claimed = due
            && self
                .inner
                .last_fetch
                .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok();
        if !claimed {
            return held.ok_or(OidcError::Unavailable);
        }
        let issuer = IssuerUrl::new(self.inner.settings.issuer.clone())
            .map_err(|_| OidcError::Unavailable)?;
        let http = self.http();
        match CoreProviderMetadata::discover_async(issuer, &http).await {
            Ok(metadata) => {
                let metadata = Arc::new(metadata);
                if let Ok(mut slot) = self.inner.held.lock() {
                    *slot = Some(Arc::clone(&metadata));
                }
                Ok(metadata)
            }
            Err(error) => {
                tracing::warn!(error = %error, "identity provider discovery failed");
                held.ok_or(OidcError::Unavailable)
            }
        }
    }

    fn client(&self, metadata: &CoreProviderMetadata) -> std::result::Result<Client, OidcError> {
        let settings = &self.inner.settings;
        let redirect =
            RedirectUrl::new(settings.redirect_uri.clone()).map_err(|_| OidcError::Unavailable)?;
        Ok(Client::from_provider_metadata(
            metadata.clone(),
            ClientId::new(settings.client_id.clone()),
            Some(ClientSecret::new(
                settings.client_secret.expose().to_owned(),
            )),
        )
        .set_redirect_uri(redirect))
    }

    /// Starts a sign-in: a fresh state, nonce and PKCE verifier, and the URL that sends the browser to the provider —
    /// asking it, with `max_age`, to make the person authenticate again if they last did longer ago than that.
    ///
    /// # Errors
    /// [`OidcError::Unavailable`] when the provider's discovery document cannot be had.
    pub async fn begin(&self, max_age: Option<u64>) -> std::result::Result<Started, OidcError> {
        let metadata = self.discovered(false).await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let client = self.client(&metadata)?;
        let mut request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);
        if let Some(seconds) = max_age {
            request = request.set_max_age(std::time::Duration::from_secs(seconds));
        }
        let (url, state, nonce) = request.url();
        Ok(Started {
            url: url.to_string(),
            state: state.secret().clone(),
            nonce: SecretKey::new(nonce.secret().clone()),
            verifier: SecretKey::new(verifier.secret().clone()),
        })
    }

    /// Finishes a sign-in whose callback carried `code` (and `iss`, when the provider sends it), checked against the
    /// `nonce` and `verifier` its start recorded.
    ///
    /// # Errors
    /// The [`OidcError`] naming the first check that failed.
    pub async fn finish(
        &self,
        code: &str,
        iss: Option<&str>,
        nonce: &SecretKey,
        verifier: &SecretKey,
    ) -> std::result::Result<ProviderIdentity, OidcError> {
        let settings = &self.inner.settings;
        if iss.is_some_and(|iss| iss != settings.issuer) {
            return Err(OidcError::IssuerMismatch);
        }
        let metadata = self.discovered(false).await?;
        let http = self.http();
        let token = self
            .client(&metadata)?
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|_| OidcError::Unavailable)?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.expose().to_owned()))
            .request_async(&http)
            .await
            .map_err(|error| {
                tracing::info!(error = %error, "identity provider refused the code");
                OidcError::CodeRefused
            })?;
        let id_token = token.id_token().ok_or(OidcError::NoIdToken)?;
        let nonce = Nonce::new(nonce.expose().to_owned());
        let verified = match verify(settings, id_token, &metadata, &nonce, token.access_token()) {
            Err(OidcError::IdTokenRefused("unknown key")) => {
                // A key the console does not hold: the provider may have rotated; ask once, then decide.
                let refreshed = self.discovered(true).await?;
                verify(settings, id_token, &refreshed, &nonce, token.access_token())
            }
            other => other,
        }
        .map_err(|error| match error {
            OidcError::IdTokenRefused("unknown key") => OidcError::IdTokenRefused("signature"),
            other => other,
        })?;
        if !verified
            .amr
            .iter()
            .any(|factor| settings.mfa_amr.contains(factor))
        {
            return Err(OidcError::NoSecondFactor);
        }
        Ok(ProviderIdentity {
            issuer: settings.issuer.clone(),
            subject: verified.subject,
            // A provider's clock ahead of ours must not make an authentication look more recent than now.
            authenticated: verified.auth_time.map(|at| at.min(now_secs())),
        })
    }
}
