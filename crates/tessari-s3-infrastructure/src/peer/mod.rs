//! An HTTP client for calls between nodes of the cluster. It knows transport and time and nothing of what is sent:
//! a connection must open within [`PEER_CONNECT_TIMEOUT_SECS`], an answer that stalls longer than
//! [`PEER_READ_TIMEOUT_SECS`] between reads is abandoned, and every call carries the budget its caller gives it.
//! It never retries — whether a call may be repeated is the caller's knowledge.

mod error;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{BoxStream, Stream, StreamExt};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tessari_s3_constants::{PEER_CONNECT_TIMEOUT_SECS, PEER_READ_TIMEOUT_SECS};

use tessari_s3_types::NodeId;

use crate::InternalTls;

pub use error::PeerError;

/// A peer, and where its internal surface answers (`host:port`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAt {
    /// The node.
    pub node: NodeId,
    /// Its internal surface.
    pub endpoint: String,
}

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

/// Where each node's internal surface answers, by the name its peer credential carries: the client dials
/// `https://<node>.peer.tessari-s3:<port>` so the TLS server name, and so the name the server's certificate is asked
/// about, is the node meant — and this resolver turns that name into the node's registered address.
#[derive(Debug, Default)]
struct Directory {
    endpoints: scc::HashMap<String, String>,
}

impl Resolve for Directory {
    fn resolve(&self, name: Name) -> Resolving {
        let endpoint = self
            .endpoints
            .read_sync(name.as_str(), |_, endpoint| endpoint.clone());
        Box::pin(async move {
            let endpoint = endpoint.ok_or("no endpoint is known for this peer")?;
            let addresses: Vec<_> = tokio::net::lookup_host(endpoint).await?.collect();
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

/// The client; cheap to clone, every clone sharing one connection pool.
#[derive(Debug, Clone)]
pub struct PeerHttp {
    client: reqwest::Client,
    directory: Arc<Directory>,
}

impl PeerHttp {
    /// A client with the cluster's connect and stall timeouts, over mutual TLS with `tls`: it presents this node's
    /// certificate and accepts only a server whose certificate the cluster authority issued for the node dialled.
    ///
    /// # Errors
    /// [`PeerError::Configuration`] when the TLS material or the HTTP stack cannot be used.
    pub fn new(tls: &InternalTls) -> Result<Self, PeerError> {
        let config = crate::tls::client_config(tls).map_err(|_| PeerError::Configuration {
            reason: "the internal TLS material cannot be used",
        })?;
        let directory = Arc::new(Directory::default());
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(PEER_CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(PEER_READ_TIMEOUT_SECS))
            .tls_backend_preconfigured(config)
            .dns_resolver(Arc::clone(&directory))
            .build()
            .map(|client| Self { client, directory })
            .map_err(|_| PeerError::Configuration {
                reason: "the HTTP client could not be built",
            })
    }

    /// Sends `method target` to `peer.node`, whose internal surface is at `peer.endpoint` (`host:port`), with `headers` and
    /// `body`, and waits for the answer's status and headers for at most `budget` — which therefore covers sending
    /// the whole body.
    ///
    /// # Errors
    /// [`PeerError::Unavailable`] when no answer arrives: refused or failed connection — a server whose certificate
    /// does not name the node included — a stall, or the budget spent; [`PeerError::Configuration`] when the
    /// endpoint is not `host:port`.
    pub async fn send(
        &self,
        method: PeerMethod,
        peer: &PeerAt,
        target: &str,
        headers: &[(&'static str, String)],
        body: PeerBody,
        budget: Duration,
    ) -> Result<PeerReply, PeerError> {
        let PeerAt { node, endpoint } = peer;
        let port = endpoint
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .ok_or(PeerError::Configuration {
                reason: "a peer endpoint is not host:port",
            })?;
        let name = crate::credential::names(node);
        self.directory
            .endpoints
            .upsert_async(name.clone(), endpoint.to_owned())
            .await;
        let url = format!("https://{name}:{port}{target}");
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
