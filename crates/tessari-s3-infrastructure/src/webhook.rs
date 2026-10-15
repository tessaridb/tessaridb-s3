//! Delivering a notification to an operator-declared webhook: one POST, bounded in time, never following a redirect
//! (a destination is an address the operator chose, and a redirect would let the receiver choose another).

use std::time::Duration;

use tessari_s3_constants::NOTIFY_DELIVERY_TIMEOUT_SECS;

use crate::{Error, Result};

/// A webhook client; cheap to clone, every clone sharing one connection pool.
#[derive(Debug, Clone)]
pub struct WebhookClient {
    client: reqwest::Client,
}

impl WebhookClient {
    /// A client trusting the platform's authorities for `https://` destinations.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] when the TLS stack or the HTTP client cannot be built.
    pub fn new() -> Result<Self> {
        let config = crate::tls::public_client_config()?;
        reqwest::Client::builder()
            .timeout(Duration::from_secs(NOTIFY_DELIVERY_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_preconfigured(config)
            .build()
            .map(|client| Self { client })
            .map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_NOTIFY_TARGETS",
                reason: "the HTTP client for webhooks cannot be built",
            })
    }

    /// Posts `body` (JSON) to `url`; `true` only for a 2xx answer.
    pub async fn post(&self, url: &str, body: bytes::Bytes) -> bool {
        match self
            .client
            .post(url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => true,
            Ok(response) => {
                tracing::warn!(
                    status = response.status().as_u16(),
                    "a webhook refused a notification"
                );
                false
            }
            Err(error) => {
                tracing::warn!(%error, "a webhook could not be reached");
                false
            }
        }
    }
}
