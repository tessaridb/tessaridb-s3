//! [`RemoteShards`]: put, read and remove one shard on another node. A call either gets the answer it needs or a
//! typed failure — the peer's refusal by status, or its absence — and is never retried here: whether a shard write
//! may be sent again, and to which node, is the coordinator's decision.

use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{BoxStream, Stream, StreamExt};
use tessari_s3_constants::{
    INTERNAL_DRIVE_BODY_MAX, INTERNAL_DRIVE_PATH, INTERNAL_HEALTH_PATH, INTERNAL_NODE_HEADER,
    INTERNAL_SHARDS_PATH,
};
use tessari_s3_infrastructure::InternalTls;
pub use tessari_s3_infrastructure::peer::PeerAt;
use tessari_s3_infrastructure::peer::{PeerBody, PeerHttp, PeerMethod, PeerReply};
use tessari_s3_types::NodeId;

use crate::data::{DriveSpace, hex};
use crate::{Error, Result};

/// Which shard: a data id and the shard's index in its stripe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardRef {
    /// The data id.
    pub id: [u8; 16],
    /// The shard index.
    pub index: u16,
}

/// Shard calls to other nodes as `node`, over mutual TLS: this node's certificate speaks for it, and a peer is
/// reached only when its certificate speaks for the node dialled.
#[derive(Clone)]
pub struct RemoteShards {
    http: PeerHttp,
    node: NodeId,
}

impl RemoteShards {
    /// Calls made as `node` with the cluster's TLS material `tls`.
    ///
    /// # Errors
    /// [`Error::Peer`] when the TLS material or the HTTP client cannot be used.
    pub fn new(node: NodeId, tls: &InternalTls) -> Result<Self> {
        Ok(Self {
            http: PeerHttp::new(tls)?,
            node,
        })
    }

    /// Stores `body` — `size` bytes in blocks of `block_size` — as `shard` on `peer`; returns once
    /// the peer has made it durable. `budget` covers sending the whole body.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] with the peer's status, or [`Error::Peer`] when it does not answer in time.
    pub async fn put<S>(
        &self,
        peer: &PeerAt,
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
            .call(PeerMethod::Put, peer, &target, body, budget)
            .await?;
        answered(&reply, 200)
    }

    /// Reads `shard` of `size` bytes from `peer`, all of it or the blocks in `blocks`; each block is
    /// verified by the peer before it is sent. The stream ending early is the peer's failure part way — the caller
    /// knows the length it asked for.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] (404: the peer holds no such shard), or [`Error::Peer`].
    pub async fn get(
        &self,
        peer: &PeerAt,
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
            .call(PeerMethod::Get, peer, &target, PeerBody::Empty, budget)
            .await?;
        answered(&reply, 200)?;
        Ok(reply
            .into_stream()
            .map(|chunk| chunk.map_err(Error::from))
            .boxed())
    }

    /// Removes `shard` from `peer`; a shard the peer does not hold is not an error.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] or [`Error::Peer`].
    pub async fn delete(&self, peer: &PeerAt, shard: ShardRef, budget: Duration) -> Result<()> {
        let reply = self
            .call(
                PeerMethod::Delete,
                peer,
                &path(shard),
                PeerBody::Empty,
                budget,
            )
            .await?;
        answered(&reply, 204)
    }

    /// Asks `peer` whether it is up: it answers only an admitted member presenting its own
    /// certificate, so an answer within `budget` means it is serving and admits this node.
    ///
    /// # Errors
    /// [`Error::PeerRefused`] or [`Error::Peer`].
    pub async fn probe(&self, peer: &PeerAt, budget: Duration) -> Result<()> {
        let reply = self
            .call(
                PeerMethod::Get,
                peer,
                INTERNAL_HEALTH_PATH,
                PeerBody::Empty,
                budget,
            )
            .await?;
        answered(&reply, 204)
    }

    /// The space of the data drive of `peer`; `None` when it has no data directory.
    ///
    /// # Errors
    /// [`Error::PeerRefused`], [`Error::Peer`], or [`Error::Malformed`] for an answer not in the drive form.
    pub async fn drive(&self, peer: &PeerAt, budget: Duration) -> Result<Option<DriveSpace>> {
        let reply = self
            .call(
                PeerMethod::Get,
                peer,
                INTERNAL_DRIVE_PATH,
                PeerBody::Empty,
                budget,
            )
            .await?;
        if reply.status() == 404 {
            return Ok(None);
        }
        answered(&reply, 200)?;
        let mut body = Vec::new();
        let mut chunks = reply.into_stream();
        while let Some(chunk) = chunks.next().await {
            body.extend_from_slice(&chunk?);
            if body.len() > INTERNAL_DRIVE_BODY_MAX {
                break;
            }
        }
        let malformed = Error::Malformed {
            record: "drive",
            reason: "a peer's drive answer is not three byte counts",
        };
        let text = std::str::from_utf8(&body).map_err(|_| malformed.clone())?;
        DriveSpace::from_wire(text).map(Some).ok_or(malformed)
    }

    async fn call(
        &self,
        method: PeerMethod,
        peer: &PeerAt,
        target: &str,
        body: PeerBody,
        budget: Duration,
    ) -> Result<PeerReply> {
        // The certificate proves who calls; the header says who it claims to be, and the peer asks the one about the
        // other.
        let headers = [(INTERNAL_NODE_HEADER, self.node.as_str().to_owned())];
        Ok(self
            .http
            .send(method, peer, target, &headers, body, budget)
            .await?)
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
