//! Requests to the identity provider: bounded in time and in size, never following a redirect (the provider's
//! addresses come from its discovery document under the configured issuer, and a redirect would let anybody on the
//! way choose another).

use std::time::Duration;

use openidconnect::http::{HeaderMap, Response};
use openidconnect::{HttpRequest, HttpResponse};
use tessari_s3_constants::{CONSOLE_OIDC_HTTP_TIMEOUT_SECS, CONSOLE_OIDC_RESPONSE_MAX_BYTES};

use crate::{Error, Result};

/// Why a request to the identity provider brought back no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProviderHttpError {
    /// Not reachable, timed out, or the connection broke.
    #[error("the identity provider could not be reached")]
    Unreachable,
    /// The answer was longer than the console reads.
    #[error("the identity provider's answer was too large")]
    TooLarge,
}

/// The HTTP client the identity provider is asked through; cheap to clone.
#[derive(Debug, Clone)]
pub(crate) struct ProviderHttp {
    client: reqwest::Client,
}

impl ProviderHttp {
    /// A client trusting the platform's authorities for `https://` addresses.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] when the TLS stack or the HTTP client cannot be built.
    pub(crate) fn new() -> Result<Self> {
        let config = crate::tls::public_client_config()?;
        reqwest::Client::builder()
            .timeout(Duration::from_secs(CONSOLE_OIDC_HTTP_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_preconfigured(config)
            .build()
            .map(|client| Self { client })
            .map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
                reason: "the HTTP client for the identity provider cannot be built",
            })
    }

    /// Sends `request` and reads its answer, at most [`CONSOLE_OIDC_RESPONSE_MAX_BYTES`] of body.
    pub(crate) async fn send(
        &self,
        request: HttpRequest,
    ) -> std::result::Result<HttpResponse, ProviderHttpError> {
        let (parts, body) = request.into_parts();
        let mut response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|_| ProviderHttpError::Unreachable)?;
        let status = response.status();
        let headers: HeaderMap = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ProviderHttpError::Unreachable)?
        {
            let total = bytes
                .len()
                .checked_add(chunk.len())
                .ok_or(ProviderHttpError::TooLarge)?;
            if total > CONSOLE_OIDC_RESPONSE_MAX_BYTES {
                return Err(ProviderHttpError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut answer = Response::new(bytes);
        *answer.status_mut() = status;
        *answer.headers_mut() = headers;
        Ok(answer)
    }
}
