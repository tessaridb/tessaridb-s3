//! Sending a replica to its destination region's S3 surface: one signed request, never following a redirect (the
//! endpoint is the one the destination region advertised), with the body streamed rather than held in memory.

use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tessari_s3_constants::REPLICATION_CONNECT_TIMEOUT_SECS;

use crate::{Error, Result};

/// A replica client; cheap to clone, every clone sharing one connection pool.
#[derive(Debug, Clone)]
pub struct ReplicaClient {
    client: reqwest::Client,
}

impl ReplicaClient {
    /// A client trusting the platform's authorities for `https://` endpoints.
    ///
    /// # Errors
    /// [`Error::InvalidConfig`] when the TLS stack or the HTTP client cannot be built.
    pub fn new() -> Result<Self> {
        let config = crate::tls::public_client_config()?;
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(REPLICATION_CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_preconfigured(config)
            .build()
            .map(|client| Self { client })
            .map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_ADVERTISE",
                reason: "the HTTP client for replication cannot be built",
            })
    }

    /// Sends `method` to `url` with `headers` (already signed) and, for a PUT, `body`; the answer's status, or why
    /// none came back.
    ///
    /// # Errors
    /// The endpoint could not be reached or the exchange broke off, described for the operator's log.
    pub async fn send<S>(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<S>,
    ) -> std::result::Result<(u16, String), String>
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        let method =
            reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?;
        let mut request = self.client.request(method, url);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(body) = body {
            request = request.body(reqwest::Body::wrap_stream(body));
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        Ok((status, text))
    }
}
