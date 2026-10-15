//! What moving data onto a new layout asks of the metadata store: the ids still on an older layout, the switch of one
//! id to the new layout with the record of what to retire, and the retires that are due.

use crate::Result;

/// Layout-move records in the metadata store.
pub(crate) trait LayoutMoveRepository: Send + Sync {
    /// Up to `limit` data ids whose shards were placed under a layout older than `current`.
    fn older(
        &self,
        current: u32,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<[u8; 16]>>> + Send;
    /// Switches data `id` from layout `old` to `new` and records the old placement for retiring, in one transaction;
    /// `false` when nothing was written — the id is no longer on `old` (moved or gone meanwhile), or a retire from its
    /// previous move is still pending, whose shards a second record would lose track of.
    fn switch(&self, id: [u8; 16], old: u32, new: u32)
    -> impl Future<Output = Result<bool>> + Send;
    /// Up to `limit` retires switched more than `grace_secs` ago: each data id and the layout it left.
    fn due_retires(
        &self,
        grace_secs: u64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<([u8; 16], u32)>>> + Send;
    /// Drops the retire record of data `id`.
    fn retired(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
}
