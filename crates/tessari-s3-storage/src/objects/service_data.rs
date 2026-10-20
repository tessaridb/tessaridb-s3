//! The data side of object operations: a data id's bytes — a file on this node's drive, or on a cluster member k+m
//! shards over the cluster — from the moment the id is queued for reclamation to the moment they are reclaimed.

use std::sync::Arc;

use futures_util::StreamExt;
use tessari_s3_constants::{DATA_BLOCK_SIZE, RECLAIM_CLAIM_BATCH, RECLAIM_WORKERS};

use super::ObjectService;
use crate::data::hex;
use crate::objects::ObjectReader;
use crate::objects::model::{HealBacklog, Healed, MoveBacklog, Moved, Reclaimed};
use crate::objects::repository::ObjectRepository;
use crate::objects::upload::Upload;
use crate::{Error, Result};

/// What reclaiming one entry came to.
enum Settled {
    /// Its bytes and its entry are gone.
    Removed,
    /// An object references it: the entry is gone, the bytes kept.
    Kept,
    /// A commit took it, or another node holds it: the entry stays for later.
    Left,
}

impl ObjectService {
    /// Starts writing `size` bytes under a new data id: the id is queued for reclamation first (on a cluster member
    /// with its layout), then the file or the shards are created.
    ///
    /// # Errors
    /// [`Error::NoLayout`], the metadata store's refusal or outage, or the drive's.
    pub async fn upload(&self, size: u64) -> Result<Upload> {
        self.start(*uuid::Uuid::new_v4().as_bytes(), size, false)
            .await
    }

    /// Bytes per stored block: an erasure stripe on a cluster member, else a data file block.
    pub(super) fn stored_block(&self) -> u32 {
        if self.erasure.is_some() {
            DATA_BLOCK_SIZE
        } else {
            self.data.block_size()
        }
    }

    /// Starts writing `size` stored bytes under data id `id`, a sealed file's header when `sealed`.
    pub(super) async fn start(&self, id: [u8; 16], size: u64, sealed: bool) -> Result<Upload> {
        if let Some(writes) = &self.erasure {
            return match writes.start(id, size, sealed).await {
                Ok(upload) => Ok(Upload::erasure(id, upload, Arc::clone(writes))),
                Err(error) => {
                    self.release_logged(id).await;
                    Err(error)
                }
            };
        }
        self.repository.queue(id).await?;
        let created = if sealed {
            self.data.create_sealed(id).await
        } else {
            self.data.create(id).await
        };
        match created {
            Ok(writer) => Ok(Upload::new(id, writer)),
            Err(error) => {
                self.release_logged(id).await;
                Err(error)
            }
        }
    }

    /// Removes the temporary files of writes a crash cut short, and answers how many.
    ///
    /// # Errors
    /// The drive's refusal.
    pub async fn recover(&self) -> Result<usize> {
        self.data.recover().await
    }

    /// Removes data `id`'s bytes and then its queue entry; a failure leaves the entry for the reclaimer.
    ///
    /// # Errors
    /// The drive's, a peer's or the metadata store's failure.
    pub async fn release(&self, id: [u8; 16]) -> Result<()> {
        self.remove_data(id).await?;
        self.repository.unqueue(id).await
    }

    /// Removes data `id`'s bytes: every shard from every node when it is erasure-coded, else its file.
    async fn remove_data(&self, id: [u8; 16]) -> Result<()> {
        if let Some(writes) = &self.erasure
            && writes.sharded(id).await?.is_some()
        {
            return writes.remove(id).await;
        }
        self.data.remove(id).await
    }

    /// Whether data `id` is still queued and unclaimed, so a commit could still take it.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub(crate) async fn claimable(&self, id: [u8; 16]) -> Result<bool> {
        self.repository.claimable(id).await
    }

    pub(super) async fn release_logged(&self, id: [u8; 16]) {
        if let Err(error) = self.release(id).await {
            let data = uuid::Uuid::from_bytes(id);
            tracing::warn!(%data, error = %error, "data left queued for the reclaimer");
        }
    }

    /// One reclamation pass over at most `limit` queue entries past their grace, taken from the queue so no other
    /// node works on them meanwhile; on a cluster member it also removes shard files older than `grace_secs` that no
    /// data id queued or committed.
    ///
    /// # Errors
    /// The metadata store's or the drive's failure; the entries this pass held go back to the queue when their
    /// hold lapses.
    pub async fn reclaim(&self, grace_secs: u64, limit: usize) -> Result<Reclaimed> {
        let mut done = Reclaimed::default();
        let mut refused = None;
        // Entries are claimed a batch at a time in one statement, so a pass meets the other nodes' claims and the
        // writers queuing beside them a few times rather than once per entry; the batch is worked
        // `RECLAIM_WORKERS` at a time. A refusal ends the pass after its batch; the entry it hit is held until its
        // hold lapses.
        while done.examined < limit && refused.is_none() {
            let batch = self
                .repository
                .claim_due(RECLAIM_CLAIM_BATCH.min(limit.saturating_sub(done.examined)))
                .await?;
            if batch.is_empty() {
                break;
            }
            done.examined = done.examined.saturating_add(batch.len());
            let mut settled = futures_util::stream::iter(batch)
                .map(|(id, marked)| self.reclaim_one(id, marked))
                .buffer_unordered(RECLAIM_WORKERS);
            while let Some(outcome) = settled.next().await {
                match outcome {
                    Ok(Settled::Removed) => done.removed = done.removed.saturating_add(1),
                    Ok(Settled::Kept) => done.kept = done.kept.saturating_add(1),
                    Ok(Settled::Left) => {}
                    Err(error) => refused = refused.or(Some(error)),
                }
            }
        }
        if let Some(error) = refused {
            return Err(error);
        }
        if let Some(writes) = &self.erasure {
            done.orphans = writes.sweep(grace_secs, limit).await?;
        }
        Ok(done)
    }

    /// Reclaims one held entry: its bytes and then the entry, unless an object references it or a commit took it.
    async fn reclaim_one(&self, id: [u8; 16], marked: bool) -> Result<Settled> {
        let data = uuid::Uuid::from_bytes(id);
        if self.repository.referenced(id).await? {
            // Not a state this server writes; keep the bytes and say so.
            tracing::warn!(%data, "a queued data file is referenced by an object; kept");
            self.repository.unqueue(id).await?;
            return Ok(Settled::Kept);
        }
        // The mark is what a racing commit asserts against: from here on the file is the reclaimer's. A false mark
        // means the commit got there first.
        if !marked && !self.repository.mark(id).await? {
            return Ok(Settled::Left);
        }
        match self.remove_data(id).await {
            // Another node is healing it; the entry stays marked and goes back to the queue for a later pass, behind
            // the entries waiting after it.
            Err(Error::Held) => {
                self.repository.defer(id).await?;
                return Ok(Settled::Left);
            }
            removed => removed?,
        }
        self.repository.unqueue(id).await?;
        Ok(Settled::Removed)
    }

    /// Rewrites the missing or damaged shards of up to `limit` data ids listed for healing.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn heal(&self, limit: usize) -> Result<Healed> {
        match &self.erasure {
            Some(writes) => writes.heal_pass(limit).await,
            None => Ok(Healed::default()),
        }
    }

    /// Moves up to `limit` data ids written under an older layout onto the current one, and first removes the shards
    /// earlier moves left behind once they are `grace_secs` old, copying at most `budget` bytes a second; nothing off
    /// a cluster.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn move_layouts(
        &self,
        grace_secs: u64,
        limit: usize,
        budget: std::num::NonZeroU64,
    ) -> Result<Moved> {
        match &self.erasure {
            Some(writes) => writes.move_pass(grace_secs, limit, budget).await,
            None => Ok(Moved::default()),
        }
    }

    /// How many data ids are still on a layout older than the current one, counted up to `limit`; `None` off a
    /// cluster, where nothing is.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn moving(&self, limit: usize) -> Result<Option<MoveBacklog>> {
        match &self.erasure {
            Some(writes) => writes.moving(limit).await.map(Some),
            None => Ok(None),
        }
    }

    /// Reads of this node's data files that failed verification since it started: a block hash, a header or a length.
    #[must_use]
    pub fn verification_failures(&self) -> u64 {
        self.data.verification_failures()
    }

    /// How many data ids are listed for healing, counted up to `limit`; `None` off a cluster, where nothing is.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn heal_backlog(&self, limit: usize) -> Result<Option<HealBacklog>> {
        match &self.erasure {
            Some(writes) => writes.backlog(limit).await.map(Some),
            None => Ok(None),
        }
    }

    /// Opens data `id` holding an object of `size` bytes, its header and length checked.
    ///
    /// # Errors
    /// [`Error::Corrupt`] (also when a committed object's data is missing — on a cluster
    /// member that includes a forgotten shard set) or the drive's failure; for erasure-coded data the store's refusal
    /// or a layout this node does not hold, and per block [`Error::Unreadable`].
    pub async fn open(&self, id: [u8; 16], size: u64) -> Result<ObjectReader> {
        self.open_stored(id, size, false).await
    }

    /// Opens data `id` holding `size` stored bytes, a sealed file's header expected when `sealed`.
    pub(super) async fn open_stored(
        &self,
        id: [u8; 16],
        size: u64,
        sealed: bool,
    ) -> Result<ObjectReader> {
        if let Some(writes) = &self.erasure
            && let Some(reader) = writes.open(id, size, sealed).await?
        {
            return Ok(ObjectReader::Erasure(reader));
        }
        let opened = if sealed {
            self.data.open_sealed(id, size).await
        } else {
            self.data.open(id, size).await
        };
        match opened {
            Ok(reader) => Ok(ObjectReader::File(reader)),
            // Only committed objects are opened here, so missing data is loss, never absence.
            Err(Error::DataIo {
                kind: std::io::ErrorKind::NotFound,
                ..
            }) => Err(Error::Corrupt {
                id: hex(id),
                reason: "the data of a committed object is missing",
            }),
            Err(error) => Err(error),
        }
    }
}
