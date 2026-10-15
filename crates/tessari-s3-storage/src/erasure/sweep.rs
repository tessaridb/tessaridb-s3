//! The orphan sweep: shard bytes on this node whose data id has no shard set any more. A shard set is recorded before
//! any of its shards exists and forgotten only after they are all removed, so a shard without one is left by a write
//! that outran a removal (a heal whose claim lapsed); nothing can read it, and the sweep removes it once it is older
//! than the reclamation grace.

use std::collections::BTreeMap;
use std::time::Duration;

use super::repository::ShardSetRepository;
use super::writes::ErasureWrites;
use crate::Result;

impl ErasureWrites {
    /// Removes the shards on this node, older than `grace_secs`, of data ids with no shard set; looks at up to
    /// `limit` shards and answers how many data ids it swept.
    ///
    /// # Errors
    /// The metadata store's or the drive's failure.
    pub(crate) async fn sweep(&self, grace_secs: u64, limit: usize) -> Result<usize> {
        let mut by_id: BTreeMap<[u8; 16], Vec<u16>> = BTreeMap::new();
        for (id, index) in self
            .shards()
            .older_than(Duration::from_secs(grace_secs), limit)
            .await?
        {
            by_id.entry(id).or_default().push(index);
        }
        let mut swept = 0_usize;
        for (id, indices) in by_id {
            if self.sets.shard_set(id).await?.is_some() {
                continue;
            }
            tracing::warn!(data = %crate::data::hex(id), "removing shard bytes with no shard set");
            for index in indices {
                self.shards().remove(id, index).await?;
            }
            swept = swept.saturating_add(1);
        }
        Ok(swept)
    }
}
