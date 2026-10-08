//! Healing: a data id listed in `heals` has every shard checked — present at its full length, every block verified —
//! and each missing or damaged one is rebuilt from the others stripe by stripe and written where the placement puts
//! it, through the same commit as any shard. A shard that verifies is never rewritten.

use std::sync::Arc;

use bytes::Bytes;
use tessari_s3_constants::SHARD_QUEUE_BLOCKS;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::read::ErasureReader;
use super::repository::ShardSetRepository;
use super::senders::{self, ShardShape};
use super::upload::Destination;
use super::writes::ErasureWrites;
use crate::objects::Healed;
use crate::peers::ShardRef;
use crate::{Error, Result};

/// What healing one data id came to.
enum Outcome {
    /// Every shard is whole, or the data is gone: the id leaves the list.
    Whole,
    /// Fewer shards verify than the read quorum: the id stays listed.
    AtRisk,
}

impl ErasureWrites {
    /// Heals up to `limit` listed data ids, each under this node's claim on it; an id another node holds is left to
    /// that node, and an id whose healing fails part way stays listed for the next pass.
    ///
    /// # Errors
    /// The metadata store's refusal or outage while listing, claiming or unlisting.
    pub(crate) async fn heal_pass(self: &Arc<Self>, limit: usize) -> Result<Healed> {
        let mut done = Healed::default();
        for id in self.sets.healing(limit).await? {
            done.examined = done.examined.saturating_add(1);
            if !self.sets.claim(id, self.holder()).await? {
                done.held = done.held.saturating_add(1);
                continue;
            }
            let outcome = self.heal_one(id).await;
            self.unclaim_logged(id).await;
            match outcome {
                Ok(Outcome::Whole) => {
                    self.sets.healed(id).await?;
                    done.healed = done.healed.saturating_add(1);
                }
                Ok(Outcome::AtRisk) => {
                    tracing::error!(data = %crate::data::hex(id), "fewer shards verify than the read quorum");
                    done.at_risk = done.at_risk.saturating_add(1);
                }
                Err(error) => {
                    tracing::warn!(data = %crate::data::hex(id), error = %error, "healing stopped; the id stays listed");
                }
            }
        }
        Ok(done)
    }

    async fn heal_one(self: &Arc<Self>, id: [u8; 16]) -> Result<Outcome> {
        let Some((set, sources)) = self.sources_of(id).await? else {
            return Ok(Outcome::Whole);
        };
        let reader = ErasureReader::new(Arc::clone(self), id, set.size, sources.clone())?;
        let mut broken = Vec::new();
        for shard in 0..sources.len() {
            if !reader.intact(shard).await {
                broken.push(shard);
            }
        }
        if broken.is_empty() {
            return Ok(Outcome::Whole);
        }
        let intact = sources.len().saturating_sub(broken.len());
        if intact < usize::from(self.code().data()) {
            return Ok(Outcome::AtRisk);
        }
        tracing::info!(data = %crate::data::hex(id), shards = ?broken, "rebuilding shards");
        self.rebuild(&reader, &sources, &broken).await?;
        Ok(Outcome::Whole)
    }

    /// Writes shards `broken` again from the stripes `reader` decodes from the others.
    async fn rebuild(
        &self,
        reader: &ErasureReader,
        sources: &[Destination],
        broken: &[usize],
    ) -> Result<()> {
        let block_len =
            u32::try_from(reader.stripes().block_len()).map_err(|_| Error::InvalidShard {
                reason: "block length",
            })?;
        let shape = ShardShape {
            block_len,
            size: reader.shard_size(),
        };
        let stripes = reader.stripes().stripe_count(reader.size());
        let budget = self
            .stall()
            .saturating_mul(u32::try_from(stripes.saturating_add(1)).unwrap_or(u32::MAX));
        let mut writers = JoinSet::new();
        let mut queues = Vec::with_capacity(broken.len());
        for &shard in broken {
            let index = u16::try_from(shard).map_err(|_| Error::InvalidShard {
                reason: "too many shards",
            })?;
            let target = ShardRef {
                id: reader.id(),
                index,
            };
            let (queue, blocks) = mpsc::channel(SHARD_QUEUE_BLOCKS);
            match sources.get(shard) {
                Some(Destination::Local) => {
                    writers.spawn(senders::local(self.shards().clone(), target, shape, blocks));
                }
                Some(Destination::Remote(endpoint)) => {
                    let peers = self.peers().clone();
                    writers.spawn(senders::remote(
                        peers,
                        endpoint.clone(),
                        target,
                        shape,
                        blocks,
                        budget,
                    ));
                }
                None => {
                    return Err(Error::InvalidShard {
                        reason: "no source for the shard",
                    });
                }
            }
            queues.push((shard, queue));
        }
        for stripe in 0..stripes {
            let bytes = reader.read_block(stripe).await?;
            let blocks = reader
                .stripes()
                .encode(&bytes)
                .map_err(|_| Error::InvalidShard {
                    reason: "stripe does not encode",
                })?;
            for (shard, queue) in &queues {
                let block = blocks.get(*shard).cloned().unwrap_or_default();
                // A writer that stopped has already failed; its error is collected below, and the others go on.
                let _stopped = queue.send_timeout(Bytes::from(block), self.stall()).await;
            }
        }
        // Closing the queues ends each shard, which its writer then commits.
        drop(queues);
        while let Some(written) = writers.join_next().await {
            written.map_err(|_| Error::InvalidShard {
                reason: "a shard writer stopped",
            })??;
        }
        Ok(())
    }
}
