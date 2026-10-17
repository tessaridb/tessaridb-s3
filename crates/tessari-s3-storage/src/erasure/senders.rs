//! One task per shard: it takes the shard's blocks from a bounded queue and writes them to the shard's node — this
//! node's drive or a peer — ending with the shard durable or with the reason it is not.

use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc::Receiver;

use crate::Result;
pub(super) use crate::peers::ShardShape;
use crate::peers::{PeerAt, RemoteShards, ShardRef};
use crate::shards::ShardService;

/// Writes the shard to this node's drive. A queue that closes early leaves a short shard, which is abandoned.
pub(super) async fn local(
    shards: ShardService,
    shard: ShardRef,
    shape: ShardShape,
    mut blocks: Receiver<Bytes>,
) -> Result<()> {
    let mut write = shards
        .write(
            shard.id,
            shard.index,
            shape.block_len,
            shape.size,
            shape.sealed,
        )
        .await?;
    while let Some(block) = blocks.recv().await {
        if let Err(error) = write.append(&block).await {
            if let Err(abandon) = write.abandon().await {
                tracing::error!(error = %abandon, "a failed shard write could not be removed");
            }
            return Err(error);
        }
    }
    write.finish().await
}

/// Streams the shard to the peer at `endpoint`, which answers once it is durable there.
pub(super) async fn remote(
    peers: RemoteShards,
    endpoint: PeerAt,
    shard: ShardRef,
    shape: ShardShape,
    blocks: Receiver<Bytes>,
    budget: Duration,
) -> Result<()> {
    let body = futures_util::stream::unfold(blocks, |mut blocks| async move {
        blocks.recv().await.map(|block| (Ok(block), blocks))
    });
    peers.put(&endpoint, shard, shape, body, budget).await
}
