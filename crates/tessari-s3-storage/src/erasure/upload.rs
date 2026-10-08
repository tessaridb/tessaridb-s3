//! [`ErasureUpload`]: an object's bytes cut into stripes, each stripe hashed and encoded into k data and m parity
//! blocks on the blocking pool, and block j of every stripe queued for shard j's node as soon as it exists. A
//! shard's queue holds [`SHARD_QUEUE_BLOCKS`]; a shard whose node keeps it full for the stall time, or fails, is
//! given up and the others go on. The upload is acknowledged only when at least the write quorum of shards is
//! durable — never on fewer.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tessari_s3_constants::SHARD_QUEUE_BLOCKS;
use tessari_s3_core::erasure::{Code, Stripes};
use tessari_s3_core::objects::checksum::{Digests, Hashes};
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinSet;

use super::senders::{self, ShardShape};
use crate::peers::{RemoteShards, ShardRef};
use crate::shards::ShardService;
use crate::{Error, Result};

/// A stripe's encoding task did not come back, so the object's digests are lost and the upload cannot finish.
const ENCODER_STOPPED: Error = Error::InvalidShard {
    reason: "the encoder stopped",
};

/// Where one shard is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// This node's drive.
    Local,
    /// The peer whose internal surface is at this `host:port`.
    Remote(String),
}

/// What one erasure upload writes and where.
#[derive(Debug, Clone)]
pub struct ErasurePlan {
    /// The data id.
    pub id: [u8; 16],
    /// RS(k, m).
    pub code: Code,
    /// Bytes per stripe.
    pub stripe_size: u32,
    /// The object's size; exactly this many bytes are appended.
    pub size: u64,
    /// One destination per shard, in shard order.
    pub destinations: Vec<Destination>,
    /// How long a shard's node may keep its queue full before the shard is given up.
    pub stall: Duration,
}

/// An upload that reached its write quorum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasureUploaded {
    /// The data id.
    pub id: [u8; 16],
    /// Object bytes written.
    pub size: u64,
    /// The object's MD5 and checksums, computed as the bytes went through.
    pub digests: Digests,
    /// Per shard index, whether it is durable on its node.
    pub durable: Vec<bool>,
}

/// An erasure-coded write in progress. Dropping it aborts every shard still being written; what reached a drive is
/// left for reclamation by its data id.
pub struct ErasureUpload {
    id: [u8; 16],
    code: Code,
    stripes: Arc<Stripes>,
    declared: u64,
    received: u64,
    stripe: Vec<u8>,
    stripe_size: usize,
    hashes: Option<Hashes>,
    queues: Vec<Option<Sender<Bytes>>>,
    senders: JoinSet<(usize, Result<()>)>,
    stall: Duration,
}

impl ErasureUpload {
    /// Starts one shard writer per destination.
    ///
    /// # Errors
    /// [`Error::InvalidShard`] when the plan's stripe size or destinations do not fit its code.
    pub fn start(plan: ErasurePlan, shards: &ShardService, peers: &RemoteShards) -> Result<Self> {
        let invalid = |reason| Error::InvalidShard { reason };
        let stripes = Stripes::new(plan.code, plan.stripe_size)
            .map_err(|_| invalid("stripe size of zero"))?;
        if plan.destinations.len() != usize::from(plan.code.width()) {
            return Err(invalid("not one destination per shard"));
        }
        let block_len =
            u32::try_from(stripes.block_len()).map_err(|_| invalid("block past a data block"))?;
        let stripe_count = stripes.stripe_count(plan.size);
        let shape = ShardShape {
            block_len,
            size: stripe_count
                .checked_mul(u64::from(block_len))
                .ok_or(invalid("shard length overflows"))?,
        };
        // Every block may take a stall's time to cross; past that the call is abandoned.
        let budget = plan
            .stall
            .saturating_mul(u32::try_from(stripe_count.saturating_add(1)).unwrap_or(u32::MAX));
        let mut queues = Vec::with_capacity(plan.destinations.len());
        let mut senders = JoinSet::new();
        for (index, destination) in plan.destinations.into_iter().enumerate() {
            let shard = ShardRef {
                id: plan.id,
                index: u16::try_from(index).map_err(|_| invalid("too many shards"))?,
            };
            let (queue, blocks) = mpsc::channel(SHARD_QUEUE_BLOCKS);
            queues.push(Some(queue));
            match destination {
                Destination::Local => {
                    let shards = shards.clone();
                    senders.spawn(async move {
                        (index, senders::local(shards, shard, shape, blocks).await)
                    });
                }
                Destination::Remote(endpoint) => {
                    let peers = peers.clone();
                    senders.spawn(async move {
                        let sent = senders::remote(peers, endpoint, shard, shape, blocks, budget);
                        (index, sent.await)
                    });
                }
            }
        }
        let stripe_size = usize::try_from(plan.stripe_size).map_err(|_| invalid("stripe size"))?;
        Ok(Self {
            id: plan.id,
            code: plan.code,
            stripes: Arc::new(stripes),
            declared: plan.size,
            received: 0,
            stripe: Vec::with_capacity(stripe_size),
            stripe_size,
            hashes: Some(Hashes::new()),
            queues,
            senders,
            stall: plan.stall,
        })
    }

    /// Appends the next bytes of the object; each full stripe is encoded and queued before more is taken.
    ///
    /// # Errors
    /// [`Error::ShardLength`] past the declared size, or [`Error::Quorum`] once too few shards remain to reach it.
    pub async fn append(&mut self, mut bytes: &[u8]) -> Result<()> {
        let received = u64::try_from(bytes.len())
            .ok()
            .and_then(|len| self.received.checked_add(len))
            .filter(|received| *received <= self.declared);
        self.received = received.ok_or(Error::ShardLength {
            declared: self.declared,
            received: self.received.saturating_add(1),
        })?;
        while !bytes.is_empty() {
            let room = self.stripe_size.saturating_sub(self.stripe.len());
            let (now, rest) = bytes.split_at(room.min(bytes.len()));
            self.stripe.extend_from_slice(now);
            bytes = rest;
            if self.stripe.len() == self.stripe_size {
                self.flush_stripe().await?;
            }
        }
        Ok(())
    }

    /// Hashes and encodes the buffered stripe off the runtime, then queues block j for shard j. A shard whose queue
    /// stays full for the stall time, or whose writer has ended, is given up.
    async fn flush_stripe(&mut self) -> Result<()> {
        let stripe = std::mem::replace(&mut self.stripe, Vec::with_capacity(self.stripe_size));
        let mut hashes = self.hashes.take().ok_or(ENCODER_STOPPED)?;
        let stripes = Arc::clone(&self.stripes);
        let (hashes, blocks) = tokio::task::spawn_blocking(move || {
            hashes.update(&stripe);
            (hashes, stripes.encode(&stripe))
        })
        .await
        .map_err(|_| ENCODER_STOPPED)?;
        self.hashes = Some(hashes);
        let blocks = blocks.map_err(|_| Error::InvalidShard {
            reason: "a stripe that does not fit its code",
        })?;
        for (index, block) in blocks.into_iter().enumerate() {
            let Some(Some(queue)) = self.queues.get(index) else {
                continue;
            };
            if queue
                .send_timeout(Bytes::from(block), self.stall)
                .await
                .is_err()
            {
                tracing::warn!(
                    index,
                    "a shard's node stopped taking blocks; writing on without it"
                );
                if let Some(slot) = self.queues.get_mut(index) {
                    *slot = None;
                }
            }
        }
        let alive = self.queues.iter().flatten().count();
        self.reachable(alive)
    }

    fn reachable(&self, shards: usize) -> Result<()> {
        let needed = self.code.write_quorum();
        let durable = u8::try_from(shards).unwrap_or(u8::MAX);
        if durable >= needed {
            Ok(())
        } else {
            Err(Error::Quorum { durable, needed })
        }
    }

    /// Queues the last stripe, closes every queue and waits for each shard's outcome.
    ///
    /// # Errors
    /// [`Error::ShardLength`] when fewer bytes than declared arrived, or [`Error::Quorum`] when fewer than the write
    /// quorum of shards are durable.
    pub async fn finish(mut self) -> Result<ErasureUploaded> {
        if self.received != self.declared {
            return Err(Error::ShardLength {
                declared: self.declared,
                received: self.received,
            });
        }
        if !self.stripe.is_empty() {
            self.flush_stripe().await?;
        }
        self.queues.clear();
        let mut durable = vec![false; usize::from(self.code.width())];
        while let Some(joined) = self.senders.join_next().await {
            match joined {
                Ok((index, Ok(()))) => {
                    if let Some(slot) = durable.get_mut(index) {
                        *slot = true;
                    }
                }
                Ok((index, Err(error))) => {
                    tracing::warn!(index, error = %error, "a shard was not written");
                }
                Err(error) => tracing::error!(error = %error, "a shard writer ended abnormally"),
            }
        }
        self.reachable(durable.iter().filter(|done| **done).count())?;
        let digests = self.hashes.take().ok_or(ENCODER_STOPPED)?.finish();
        Ok(ErasureUploaded {
            id: self.id,
            size: self.declared,
            digests,
            durable,
        })
    }
}
