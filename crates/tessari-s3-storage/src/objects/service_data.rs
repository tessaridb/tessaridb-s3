//! The data side of object operations: a data id's bytes — a file on this node's drive, or on a cluster member k+m
//! shards over the cluster — from the moment the id is queued for reclamation to the moment they are reclaimed.

use std::sync::Arc;

use super::ObjectService;
use crate::data::{DataFiles, hex};
use crate::objects::ObjectReader;
use crate::objects::model::{Healed, Reclaimed};
use crate::objects::repository::ObjectRepository;
use crate::objects::upload::Upload;
use crate::{Error, Result};

impl ObjectService {
    fn files(&self) -> Result<&DataFiles> {
        self.data.as_ref().ok_or(Error::NoDataDirectory)
    }

    /// Starts writing `size` bytes under a new data id: the id is queued for reclamation first (on a cluster member
    /// with its layout), then the file or the shards are created.
    ///
    /// # Errors
    /// [`Error::NoDataDirectory`], [`Error::NoLayout`], the metadata store's refusal or outage, or the drive's.
    pub async fn upload(&self, size: u64) -> Result<Upload> {
        let id = *uuid::Uuid::new_v4().as_bytes();
        if let Some(writes) = &self.erasure {
            return match writes.start(id, size).await {
                Ok(upload) => Ok(Upload::erasure(id, upload, Arc::clone(writes))),
                Err(error) => {
                    self.release_logged(id).await;
                    Err(error)
                }
            };
        }
        let files = self.files()?;
        self.repository.queue(id).await?;
        match files.create(id).await {
            Ok(writer) => Ok(Upload::new(id, writer)),
            Err(error) => {
                self.release_logged(id).await;
                Err(error)
            }
        }
    }

    /// Removes the temporary files of writes a crash cut short, and answers how many; a node without a data
    /// directory holds none.
    ///
    /// # Errors
    /// The drive's refusal.
    pub async fn recover(&self) -> Result<usize> {
        match &self.data {
            Some(files) => files.recover().await,
            None => Ok(0),
        }
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
            && let Some(layout) = writes.sharded(id).await?
        {
            return writes.remove(id, layout).await;
        }
        self.files()?.remove(id).await
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

    /// One reclamation pass over at most `limit` queue entries older than `grace_secs`.
    ///
    /// # Errors
    /// The metadata store's or the drive's failure.
    pub async fn reclaim(&self, grace_secs: u64, limit: usize) -> Result<Reclaimed> {
        let due = self.repository.due(grace_secs, limit).await?;
        let mut done = Reclaimed {
            examined: due.len(),
            ..Reclaimed::default()
        };
        for (id, marked) in due {
            let data = uuid::Uuid::from_bytes(id);
            if self.repository.referenced(id).await? {
                // Not a state this server writes; keep the bytes and say so.
                tracing::warn!(%data, "a queued data file is referenced by an object; kept");
                self.repository.unqueue(id).await?;
                done.kept = done.kept.saturating_add(1);
                continue;
            }
            // The mark is what a racing commit asserts against: from here on the file is the reclaimer's. A false
            // mark means the commit got there first.
            if !marked && !self.repository.mark(id).await? {
                continue;
            }
            self.remove_data(id).await?;
            self.repository.unqueue(id).await?;
            done.removed = done.removed.saturating_add(1);
        }
        Ok(done)
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

    /// Opens data `id` holding an object of `size` bytes, its header and length checked.
    ///
    /// # Errors
    /// [`Error::NoDataDirectory`], [`Error::Corrupt`] (also when a committed object's data is missing — on a cluster
    /// member that includes a forgotten shard set) or the drive's failure; for erasure-coded data the store's refusal
    /// or a layout this node does not hold, and per block [`Error::Unreadable`].
    pub async fn open(&self, id: [u8; 16], size: u64) -> Result<ObjectReader> {
        if let Some(writes) = &self.erasure
            && let Some(reader) = writes.open(id, size).await?
        {
            return Ok(ObjectReader::Erasure(reader));
        }
        match self.files()?.open(id, size).await {
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
