//! Shard-set queries: which layout a data id's shards were placed under, and which ids need healing.

use crate::Result;

/// A data id's shard set: the layout its shards were placed under and the bytes of the object they hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShardSet {
    pub(crate) layout: u32,
    pub(crate) size: u64,
}

/// Shard-set records in the metadata store.
pub(crate) trait ShardSetRepository: Send + Sync {
    /// Queues data `id` for reclamation and records its shard set, in ONE transaction, before any shard exists.
    fn queue(&self, id: [u8; 16], set: ShardSet) -> impl Future<Output = Result<()>> + Send;
    /// Data `id`'s shard set; `None` when it is not erasure-coded (or is gone).
    fn shard_set(&self, id: [u8; 16]) -> impl Future<Output = Result<Option<ShardSet>>> + Send;
    /// Removes data `id`'s shard-set record, after every shard is gone.
    fn forget(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Lists data `id` for healing.
    fn heal(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Up to `limit` data ids listed for healing.
    fn healing(&self, limit: usize) -> impl Future<Output = Result<Vec<[u8; 16]>>> + Send;
    /// Removes data `id` from the healing list.
    fn healed(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Takes the expiring claim on data `id` for `holder` — free, or already `holder`'s; `false` when another holder
    /// has it.
    fn claim(&self, id: [u8; 16], holder: &str) -> impl Future<Output = Result<bool>> + Send;
    /// Hands back `holder`'s claim on data `id`; a claim that lapsed or passed to another holder is left alone.
    fn unclaim(&self, id: [u8; 16], holder: &str) -> impl Future<Output = Result<()>> + Send;
}
