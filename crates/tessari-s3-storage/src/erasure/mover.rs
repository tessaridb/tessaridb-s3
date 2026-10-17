//! Moving data onto the current layout: every shard the new placement puts on another node is copied there from the
//! stripes the old shards decode to, verified, and only then is the id switched to the new layout — in the same
//! transaction that records the old placement for retiring. The old shards stay until the retire is due, past the
//! reclaim grace, so a read that resolved the old placement before the switch still finds them. Every step is
//! re-derived from the store, so a pass cut short anywhere is finished by the next one, on any node.

use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{Either, select};
use tessari_s3_constants::HEAL_CLAIM_SECS;
use tessari_s3_core::erasure::Layout;

use super::moves_repository::LayoutMoveRepository;
use super::pace::Pace;
use super::read::ErasureReader;
use super::repository::ShardSetRepository;
use super::upload::Destination;
use super::writes::ErasureWrites;
use crate::objects::{MoveBacklog, Moved};
use crate::peers::ShardRef;
use crate::{Error, Result};

impl ErasureWrites {
    /// Removes the shards earlier moves left behind once their retire is `grace_secs` old, then moves up to `limit`
    /// data ids written under an older layout onto the current one; each id under this node's claim, and an id
    /// another node holds is left to it. The copies keep to `budget` bytes a second over the whole pass. A move or
    /// retire that fails part way is logged and left for the next pass.
    ///
    /// # Errors
    /// [`Error::NoLayout`], or the metadata store's refusal or outage while listing, claiming or switching.
    pub(crate) async fn move_pass(
        self: &Arc<Self>,
        grace_secs: u64,
        limit: usize,
        budget: NonZeroU64,
    ) -> Result<Moved> {
        let mut done = Moved::default();
        let mut pace = Pace::new(budget);
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
            let moved = self.move_one(id, &current, &mut pace).await;
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

    /// How many data ids are on a layout older than the current one, counted up to `limit`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn moving(&self, limit: usize) -> Result<MoveBacklog> {
        let current = match self.current().await {
            Ok(current) => current,
            Err(Error::NoLayout) => return Ok(MoveBacklog::default()),
            Err(error) => return Err(error),
        };
        let older = self
            .sets
            .older(current.version(), limit.saturating_add(1))
            .await?;
        Ok(MoveBacklog {
            listed: older.len().min(limit),
            more: older.len() > limit,
        })
    }

    /// Copies the shards of data `id` whose node changes under `current`, verifies each copy, and switches the id;
    /// `false` when the id is gone or already moved, read again under the claim.
    async fn move_one(
        self: &Arc<Self>,
        id: [u8; 16],
        current: &Layout,
        pace: &mut Pace,
    ) -> Result<bool> {
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
            let reader = ErasureReader::new(Arc::clone(self), id, set, sources)?;
            {
                let copy = std::pin::pin!(self.rebuild(&reader, &targets, &changed, Some(pace)));
                let renewal = std::pin::pin!(self.renew(id));
                // The copy can outlast the claim's lease at a low budget, so the claim is renewed while it runs; a
                // claim another node took meanwhile stops the copy, and nothing is switched.
                match select(copy, renewal).await {
                    Either::Left((copied, _)) | Either::Right((copied, _)) => copied?,
                }
            }
            let copies = ErasureReader::new(Arc::clone(self), id, set, targets)?;
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

    /// Renews this node's claim on data `id` every third of its lease, until the claim is another node's.
    async fn renew(&self, id: [u8; 16]) -> Result<()> {
        let mut ticks = tokio::time::interval(Duration::from_secs(HEAL_CLAIM_SECS / 3));
        // The first tick is immediate, and the claim was just taken.
        ticks.tick().await;
        loop {
            ticks.tick().await;
            if !self.sets.claim(id, self.holder()).await? {
                return Err(Error::Held);
            }
        }
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
