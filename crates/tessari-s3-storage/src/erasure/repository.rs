//! Shard-set queries: which layout a data id's shards were placed under, and which ids need healing.

use crate::Result;

/// Shard-set records in the metadata store.
pub(crate) trait ShardSetRepository: Send + Sync {
    /// Queues data `id` for reclamation and records `layout` as its shards' layout, in ONE transaction, before any
    /// shard exists.
    fn queue(&self, id: [u8; 16], layout: u32) -> impl Future<Output = Result<()>> + Send;
    /// The layout data `id` was placed under; `None` when it is not erasure-coded (or is gone).
    fn layout_of(&self, id: [u8; 16]) -> impl Future<Output = Result<Option<u32>>> + Send;
    /// Removes data `id`'s shard-set record, after every shard is gone.
    fn forget(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Lists data `id` for healing.
    fn heal(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
}
