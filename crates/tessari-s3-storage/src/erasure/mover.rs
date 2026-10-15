//! Moving data onto the current layout: every shard the new placement puts on another node is copied there from the
//! stripes the old shards decode to, verified, and only then is the id switched to the new layout — in the same
//! transaction that records the old placement for retiring. The old shards stay until the retire is due, past the
//! reclaim grace, so a read that resolved the old placement before the switch still finds them. Every step is
//! re-derived from the store, so a pass cut short anywhere is finished by the next one, on any node.

use std::sync::Arc;

use tessari_s3_core::erasure::Layout;

use super::moves_repository::LayoutMoveRepository;
use super::read::ErasureReader;
use super::repository::ShardSetRepository;
use super::upload::Destination;
use super::writes::ErasureWrites;
use crate::objects::Moved;
use crate::peers::ShardRef;
use crate::{Error, Result};

impl ErasureWrites {
    /// Removes the shards earlier moves left behind once their retire is `grace_secs` old, then moves up to `limit`
    /// data ids written under an older layout onto the current one; each id under this node's claim, and an id
    /// another node holds is left to it. A move or retire that fails part way is logged and left for the next pass.
    ///
    /// # Errors
    /// [`Error::NoLayout`], or the metadata store's refusal or outage while listing, claiming or switching.
    pub(crate) async fn move_pass(
        self: &Arc<Self>,
        grace_secs: u64,
        limit: usize,
    ) -> Result<Moved> {
        let mut done = Moved::default();
        for (id, old) in self.sets.due_retires(grace_secs, limit).await? {
            if !self.sets.claim(id, self.holder()).await? {
                done.held = done.held.saturating_add(1);
                continue;
            }
            let retired = self.retire(id, old).await;
            self.unclaim_logged(id).await;
            match retired {
                Ok(()) => done.retired = done.retired.saturating_add(1),
                Err(error) => {
                    tracing::warn!(data = %crate::data::hex(id), error = %error, "a retire is left for the next pass");
                }
            }
        }
        let current = self.current().await?;
        for id in self.sets.older(current.version(), limit).await? {
            done.examined = done.examined.saturating_add(1);
            if !self.sets.claim(id, self.holder()).await? {
                done.held = done.held.saturating_add(1);
                continue;
            }
            let moved = self.move_one(id, &current).await;
            self.unclaim_logged(id).await;
            match moved {
                Ok(true) => done.moved = done.moved.saturating_add(1),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(data = %crate::data::hex(id), error = %error, "a move is left for the next pass");
                }
            }
        }
        Ok(done)
    }

    /// Copies the shards of data `id` whose node changes under `current`, verifies each copy, and switches the id;
    /// `false` when the id is gone or already moved, read again under the claim.
    async fn move_one(self: &Arc<Self>, id: [u8; 16], current: &Layout) -> Result<bool> {
        let Some(set) = self.sets.shard_set(id).await? else {
            return Ok(false);
        };
        if set.layout >= current.version() {
            return Ok(false);
        }
        let old = self.version(set.layout).await?;
        let (was, will) = (self.placed(id, &old), self.placed(id, current));
        let changed: Vec<usize> = (0..will.len())
            .filter(|&index| was.get(index) != will.get(index))
            .collect();
        if !changed.is_empty() {
            let targets = self.destinations(id, current).await?;
            let sources = self.destinations(id, &old).await?;
            let reader = ErasureReader::new(Arc::clone(self), id, set.size, sources)?;
            self.rebuild(&reader, &targets, &changed).await?;
            let copies = ErasureReader::new(Arc::clone(self), id, set.size, targets)?;
            for &index in &changed {
                if !copies.intact(index).await {
                    return Err(Error::InvalidShard {
                        reason: "a moved shard does not verify",
                    });
                }
            }
        }
        self.sets.switch(id, set.layout, current.version()).await
    }

    /// Removes the shards of data `id` that layout `old` placed on a node its present layout does not use for the
    /// same shard — every one of them when the id is gone — then drops the retire record.
    async fn retire(&self, id: [u8; 16], old: u32) -> Result<()> {
        let old = self.version(old).await?;
        let kept = match self.sets.shard_set(id).await? {
            Some(set) => self.placed(id, &self.version(set.layout).await?),
            None => Vec::new(),
        };
        let was = self.placed(id, &old);
        let sources = self.destinations(id, &old).await?;
        for (index, (node, destination)) in was.iter().zip(sources).enumerate() {
            if kept.get(index) == Some(node) {
                continue;
            }
            let shard = ShardRef {
                id,
                index: u16::try_from(index).map_err(|_| Error::InvalidShard {
                    reason: "too many shards",
                })?,
            };
            match destination {
                Destination::Local => self.shards().remove(id, shard.index).await?,
                Destination::Remote(endpoint) => {
                    self.peers().delete(&endpoint, shard, self.stall()).await?;
                }
            }
        }
        self.sets.retired(id).await
    }
}
