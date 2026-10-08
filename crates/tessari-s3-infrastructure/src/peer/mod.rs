//! An HTTP client for calls between nodes of the cluster. It knows transport and time and nothing of what is sent:
//! a connection must open within [`PEER_CONNECT_TIMEOUT_SECS`], an answer that stalls longer than
//! [`PEER_READ_TIMEOUT_SECS`] between reads is abandoned, and every call carries the budget its caller gives it.
//! It never retries — whether a call may be repeated is the caller's knowledge.

mod error;

use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{BoxStream, Stream, StreamExt};
use tessari_s3_constants::{PEER_CONNECT_TIMEOUT_SECS, PEER_READ_TIMEOUT_SECS};

use crate::InternalTls;

pub use error::PeerError;

/// The methods peers call each other with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerMethod {
    /// Store.
    Put,
    /// Read.
    Get,
    /// Remove.
    Delete,
}

/// A request body: nothing, or bytes streamed as they are produced (never gathered first).
pub enum PeerBody {
    /// No body.
    Empty,
    /// A body read from `stream` as the peer takes it.
    Stream(BoxStream<'static, std::io::Result<Bytes>>),
}

/// A peer's answer: its status, and its body still on the wire.
pub struct PeerReply {
    status: u16,
    response: reqwest::Response,
}

/// The client; cheap to clone, every clone sharing one connection pool.
#[derive(Debug, Clone)]
pub struct PeerHttp {
    client: reqwest::Client,
    scheme: &'static str,
}

impl PeerHttp {
    /// A client with the cluster's connect and stall timeouts, over mutual TLS with `tls` or in the clear without.
    ///
    /// # Errors
    /// [`PeerError::Configuration`] when the TLS material or the HTTP stack cannot be used.
    pub fn new(tls: Option<&InternalTls>) -> Result<Self, PeerError> {
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(PEER_READ_TIMEOUT_SECS));
        let unusable = |_| PeerError::Configuration {
            reason: "the internal TLS material cannot be used",
        };
        let (config, scheme) = match tls {
            Some(tls) => (crate::tls::client_config(tls).map_err(unusable)?, "https"),
            None => (crate::tls::plain_client_config().map_err(unusable)?, "http"),
        };
        let builder = builder.tls_backend_preconfigured(config);
        builder
            .build()
            .map(|client| Self { client, scheme })
            .map_err(|_| PeerError::Configuration {
                reason: "the HTTP client could not be built",
            })
    }

    /// `https` over TLS, `http` in the clear.
    #[must_use]
    pub const fn scheme(&self) -> &'static str {
        self.scheme
    }

    /// Sends `method url` with `headers` and `body`, and waits for the answer's status and headers for at most
    /// `budget` — which therefore covers sending the whole body.
    ///
    /// # Errors
    /// [`PeerError::Unavailable`] when no answer arrives: refused or failed connection, a stall, or the budget spent.
    pub async fn send(
        &self,
        method: PeerMethod,
        url: &str,
        headers: &[(&'static str, String)],
        body: PeerBody,
        budget: Duration,
    ) -> Result<PeerReply, PeerError> {
        let method = match method {
            PeerMethod::Put => reqwest::Method::PUT,
            PeerMethod::Get => reqwest::Method::GET,
            PeerMethod::Delete => reqwest::Method::DELETE,
        };
        let mut request = self.client.request(method, url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        if let PeerBody::Stream(stream) = body {
            request = request.body(reqwest::Body::wrap_stream(stream));
        }
        let response = tokio::time::timeout(budget, request.send())
            .await
            .map_err(|_| PeerError::Unavailable {
                reason: "no answer within the budget",
            })?
            .map_err(unavailable)?;
        Ok(PeerReply {
            status: response.status().as_u16(),
            response,
        })
    }
}

fn unavailable(error: reqwest::Error) -> PeerError {
    let reason = if error.is_connect() {
        "the connection could not be opened"
    } else if error.is_timeout() {
        "the peer stopped answering"
    } else {
        "the exchange failed"
    };
    tracing::warn!(error = %error, reason, "peer call failed");
    PeerError::Unavailable { reason }
}

impl PeerReply {
    /// The HTTP status.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// The body as it arrives; a stall or a broken connection ends it with [`PeerError::Unavailable`].
    pub fn into_stream(self) -> impl Stream<Item = Result<Bytes, PeerError>> + Send + 'static {
        self.response
            .bytes_stream()
            .map(|chunk| chunk.map_err(unavailable))
    }
}
