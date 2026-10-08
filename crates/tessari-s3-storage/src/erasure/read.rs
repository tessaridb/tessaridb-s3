//! [`ErasureReader`]: an erasure-coded object read back stripe by stripe from any k of its shards. A shard that is
//! missing, fails its hash, or whose node does not answer is treated as missing and the next shard is asked; a read
//! that had to go past a missing shard lists the object for healing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use futures_util::TryStreamExt;
use tessari_s3_constants::DATA_BLOCK_SIZE;
use tessari_s3_core::erasure::Stripes;

use super::upload::Destination;
use super::writes::ErasureWrites;
use crate::peers::ShardRef;
use crate::{Error, Result};

/// One erasure-coded object, open for reading.
pub struct ErasureReader {
    writes: Arc<ErasureWrites>,
    id: [u8; 16],
    size: u64,
    stripes: Stripes,
    shard_size: u64,
    sources: Vec<Destination>,
    /// Set once this reader has listed the object for healing.
    listed: AtomicBool,
}

impl ErasureReader {
    /// A reader of data `id` holding `size` bytes, its shard j at `sources[j]`.
    pub(super) fn new(
        writes: Arc<ErasureWrites>,
        id: [u8; 16],
        size: u64,
        sources: Vec<Destination>,
    ) -> Result<Self> {
        let stripes =
            Stripes::new(writes.code(), DATA_BLOCK_SIZE).map_err(|_| Error::InvalidShard {
                reason: "stripe geometry",
            })?;
        let shard_size = u64::try_from(stripes.block_len())
            .ok()
            .and_then(|block| stripes.stripe_count(size).checked_mul(block))
            .ok_or(Error::InvalidShard {
                reason: "shard length overflows",
            })?;
        Ok(Self {
            writes,
            id,
            size,
            stripes,
            shard_size,
            sources,
            listed: AtomicBool::new(false),
        })
    }

    /// The object's size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Bytes per stripe, the unit [`Self::read_block`] reads.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        DATA_BLOCK_SIZE
    }

    /// Stripe `index` of the object, rebuilt from k verified shard blocks.
    ///
    /// # Errors
    /// [`Error::Unreadable`] when fewer than k shards answer with a verified block; [`Error::InvalidShard`] for a
    /// stripe past the object.
    pub async fn read_block(&self, index: u64) -> Result<Vec<u8>> {
        let len = self.stripe_len(index)?;
        let code = self.writes.code();
        let (data, width) = (usize::from(code.data()), usize::from(code.width()));
        let mut blocks: Vec<Option<Vec<u8>>> = vec![None; width];
        let (mut have, mut next, mut missing) = (0_usize, 0_usize, false);
        // Ask as many shards as are still needed, all at once; each failure is replaced by the next shard.
        while have < data && next < width {
            let wave: Vec<usize> = (next..width).take(data.saturating_sub(have)).collect();
            next = next.saturating_add(wave.len());
            let answers =
                futures_util::future::join_all(wave.iter().map(|&shard| self.block(shard, index)))
                    .await;
            for (shard, answer) in wave.into_iter().zip(answers) {
                match answer {
                    Ok(block) => {
                        if let Some(slot) = blocks.get_mut(shard) {
                            *slot = Some(block);
                            have = have.saturating_add(1);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(shard, stripe = index, error = %error, "a shard block is missing");
                        missing = true;
                    }
                }
            }
        }
        if missing {
            self.list_for_healing().await;
        }
        if have < data {
            return Err(Error::Unreadable {
                have: u8::try_from(have).unwrap_or(u8::MAX),
                needed: code.data(),
            });
        }
        self.stripes
            .decode(&mut blocks, len)
            .map_err(|_| Error::InvalidShard {
                reason: "stripe does not decode",
            })
    }

    /// The bytes stripe `index` holds: a whole stripe, or what is left of the object.
    fn stripe_len(&self, index: u64) -> Result<usize> {
        let start = index
            .checked_mul(u64::from(DATA_BLOCK_SIZE))
            .filter(|start| *start < self.size)
            .ok_or(Error::InvalidShard {
                reason: "stripe past the object",
            })?;
        let len = self
            .size
            .saturating_sub(start)
            .min(u64::from(DATA_BLOCK_SIZE));
        usize::try_from(len).map_err(|_| Error::InvalidShard {
            reason: "stripe length",
        })
    }

    /// Block `stripe` of shard `shard`, verified: by its hash here, or by the peer before it was sent.
    async fn block(&self, shard: usize, stripe: u64) -> Result<Vec<u8>> {
        let index = u16::try_from(shard).map_err(|_| Error::InvalidShard {
            reason: "too many shards",
        })?;
        let source = self.sources.get(shard).ok_or(Error::InvalidShard {
            reason: "no source for the shard",
        })?;
        match source {
            Destination::Local => {
                let reader = self
                    .writes
                    .shards()
                    .open(self.id, index, self.shard_size)
                    .await?;
                reader.read_block(stripe).await
            }
            Destination::Remote(endpoint) => {
                let blocks = stripe..stripe.saturating_add(1);
                let shard = ShardRef { id: self.id, index };
                let parts: Vec<Bytes> = self
                    .writes
                    .peers()
                    .get(
                        endpoint,
                        shard,
                        self.shard_size,
                        Some(blocks),
                        self.writes.stall(),
                    )
                    .await?
                    .try_collect()
                    .await?;
                let block = parts.concat();
                let expected = self.stripes.block_len();
                if block.len() == expected {
                    Ok(block)
                } else {
                    Err(Error::ShardLength {
                        declared: u64::try_from(expected).unwrap_or(u64::MAX),
                        received: u64::try_from(block.len()).unwrap_or(u64::MAX),
                    })
                }
            }
        }
    }

    /// Lists the object for healing, once per reader; a failure to list does not fail the read.
    async fn list_for_healing(&self) {
        // A flag only this reader reads: no other memory is published through it.
        if self.listed.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Err(error) = self.writes.heal(self.id).await {
            tracing::warn!(error = %error, "an object read past a missing shard could not be listed for healing");
        }
    }
}
