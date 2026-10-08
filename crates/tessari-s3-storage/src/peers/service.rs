//! [`RemoteShards`]: put, read and remove one shard on another node. A call either gets the answer it needs or a
//! typed failure — the peer's refusal by status, or its absence — and is never retried here: whether a shard write
//! may be sent again, and to which node, is the coordinator's decision.

use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{BoxStream, Stream, StreamExt};
use tessari_s3_constants::{
    INTERNAL_DATE_HEADER, INTERNAL_NODE_HEADER, INTERNAL_SHARDS_PATH, INTERNAL_SIGNATURE_HEADER,
};
use tessari_s3_core::internal::{InternalRequest, sign};
use tessari_s3_infrastructure::InternalTls;
use tessari_s3_infrastructure::peer::{PeerBody, PeerHttp, PeerMethod, PeerReply};
use tessari_s3_types::{NodeId, SecretKey};

use crate::data::hex;
use crate::{Error, Result};

/// Which shard: a data id and the shard's index in its stripe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardRef {
    /// The data id.
    pub id: [u8; 16],
    /// The shard index.
    pub index: u16,
}

/// Shard calls to other nodes, signed as `node` with the cluster secret.
#[derive(Clone)]
pub struct RemoteShards {
    http: PeerHttp,
    node: NodeId,
    secret: SecretKey,
}

impl RemoteShards {
    /// Calls made as `node`, signed with `secret`, over mutual TLS with `tls` or in the clear without.
    ///
    /// # Errors
    /// [`Error::Peer`] when the TLS material or the HTTP client cannot be used.
    pub fn new(node: NodeId, secret: SecretKey, tls: Option<&InternalTls>) -> Result<Self> {
        Ok(Self {
            http: PeerHttp::new(tls)?,
            node,
            secret,
        })
    }

    /// Stores `body` — `size` bytes in blocks of `block_size` — as `shard` on the node at `endpoint`; returns once
    /// the peer has made it durable. `budget` covers sending the whole body.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] with the peer's status, or [`Error::Peer`] when it does not answer in time.
    pub async fn put<S>(
        &self,
        endpoint: &str,
        shard: ShardRef,
        block_size: u32,
        size: u64,
        body: S,
        budget: Duration,
    ) -> Result<()>
    where
        S: Stream<Item = std::io::Result<Bytes>> + Send + 'static,
    {
        let target = format!("{}?block={block_size}&size={size}", path(shard));
        let body = PeerBody::Stream(body.boxed());
        let reply = self
            .call(PeerMethod::Put, endpoint, &target, body, budget)
            .await?;
        answered(&reply, 200)
    }

    /// Reads `shard` of `size` bytes from the node at `endpoint`, all of it or the blocks in `blocks`; each block is
    /// verified by the peer before it is sent. The stream ending early is the peer's failure part way — the caller
    /// knows the length it asked for.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] (404: the peer holds no such shard), or [`Error::Peer`].
    pub async fn get(
        &self,
        endpoint: &str,
        shard: ShardRef,
        size: u64,
        blocks: Option<Range<u64>>,
        budget: Duration,
    ) -> Result<BoxStream<'static, Result<Bytes>>> {
        let range = blocks.map_or_else(String::new, |blocks| {
            format!("&from={}&to={}", blocks.start, blocks.end)
        });
        let target = format!("{}?size={size}{range}", path(shard));
        let reply = self
            .call(PeerMethod::Get, endpoint, &target, PeerBody::Empty, budget)
            .await?;
        answered(&reply, 200)?;
        Ok(reply
            .into_stream()
            .map(|chunk| chunk.map_err(Error::from))
            .boxed())
    }

    /// Removes `shard` from the node at `endpoint`; a shard the peer does not hold is not an error.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] or [`Error::Peer`].
    pub async fn delete(&self, endpoint: &str, shard: ShardRef, budget: Duration) -> Result<()> {
        let reply = self
            .call(
                PeerMethod::Delete,
                endpoint,
                &path(shard),
                PeerBody::Empty,
                budget,
            )
            .await?;
        answered(&reply, 204)
    }

    async fn call(
        &self,
        method: PeerMethod,
        endpoint: &str,
        target: &str,
        body: PeerBody,
        budget: Duration,
    ) -> Result<PeerReply> {
        let date = now();
        let signature = sign(
            self.secret.expose().as_bytes(),
            &InternalRequest {
                method: match method {
                    PeerMethod::Put => "PUT",
                    PeerMethod::Get => "GET",
                    PeerMethod::Delete => "DELETE",
                },
                path: target,
                date,
                node: self.node.as_str(),
            },
        );
        let headers = [
            (INTERNAL_NODE_HEADER, self.node.as_str().to_owned()),
            (INTERNAL_DATE_HEADER, date.to_string()),
            (INTERNAL_SIGNATURE_HEADER, signature),
        ];
        let url = format!("{}://{endpoint}{target}", self.http.scheme());
        Ok(self.http.send(method, &url, &headers, body, budget).await?)
    }
}

/// The path of `shard` on any node.
fn path(shard: ShardRef) -> String {
    format!("{INTERNAL_SHARDS_PATH}/{}/{}", hex(shard.id), shard.index)
}

fn answered(reply: &PeerReply, expected: u16) -> Result<()> {
    if reply.status() == expected {
        Ok(())
    } else {
        tracing::warn!(status = reply.status(), "a peer refused a shard request");
        Err(Error::PeerRefused {
            status: reply.status(),
        })
    }
}

/// Seconds since the epoch by this node's clock; the peer judges the request's date against its own.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(i64::MAX)
}
