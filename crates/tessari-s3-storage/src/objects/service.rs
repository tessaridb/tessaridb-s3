//! Object rules: a write commits only into a live bucket's current incarnation, under its condition, as one
//! statement; a read sees an object only if it belongs to the bucket's current incarnation. A data file is queued for
//! reclamation before it exists and leaves the queue only in the transaction that commits an object pointing at it.

use tessari_s3_constants::{DELETE_OBJECTS_CONCURRENCY, MULTIPART_READ_ATTEMPTS};
use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::read;
use super::model::{Content, NewObject, Removed, RemovedIf, StoredObject, WriteCondition, Written};
use super::repository::{Guard, ObjectRepository, Snapshot, Wrote};
use super::tessaridb::TessariObjects;
use std::sync::Arc;

use crate::data::DataFiles;
use crate::erasure::ErasureWrites;
use crate::{Error, Result};

#[path = "service_data.rs"]
mod data;
#[path = "list.rs"]
mod list;

/// Object operations.
#[derive(Clone)]
pub struct ObjectService {
    repository: TessariObjects,
    data: Option<DataFiles>,
    erasure: Option<Arc<ErasureWrites>>,
}

impl ObjectService {
    /// The service over `repository`, with data files in `data` when a data directory is configured.
    #[must_use]
    pub(crate) const fn new(repository: TessariObjects, data: Option<DataFiles>) -> Self {
        Self {
            repository,
            data,
            erasure: None,
        }
    }

    /// The same service on a cluster member: every data id is written as shards through `writes`.
    #[must_use]
    pub(crate) fn clustered(mut self, writes: Arc<ErasureWrites>) -> Self {
        self.erasure = Some(writes);
        self
    }

    /// Whether objects above the inline size can be stored.
    #[must_use]
    pub fn stores_data(&self) -> bool {
        self.data.is_some() || self.erasure.is_some()
    }

    /// Commits `object` at `bucket/key` under `condition`. A data object whose write is refused releases its file;
    /// one whose outcome is unknown (the store did not answer) keeps it queued, because the commit may have
    /// happened.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn put(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        object: &NewObject,
        condition: &WriteCondition,
    ) -> Result<Written> {
        let outcome = self.commit(bucket, key, object, condition).await?;
        if let (Content::Data(id), false) =
            (&object.content, matches!(outcome, Written::Committed(_)))
        {
            // A refusal may be the reclaimer's, not the condition's: the file is then already gone.
            if !self.repository.claimable(*id).await? {
                return Ok(Written::DataReclaimed);
            }
        }
        if let (Content::Data(id), false) =
            (&object.content, matches!(outcome, Written::Committed(_)))
        {
            self.release_logged(*id).await;
        }
        Ok(outcome)
    }

    async fn commit(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        object: &NewObject,
        condition: &WriteCondition,
    ) -> Result<Written> {
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(Written::NoSuchBucket);
        };
        let guard = match condition {
            WriteCondition::None => Guard::None,
            WriteCondition::IfNoneMatch => Guard::Absent,
            WriteCondition::IfMatch(etag) => Guard::Etag(etag.clone()),
        };
        match self
            .repository
            .write(bucket, key, incarnation, object, &guard)
            .await?
        {
            Wrote::Record(value) => Ok(Written::Committed(read(&value)?.1.modified)),
            Wrote::Refused => {
                self.classify_refusal(bucket, key, incarnation, object, condition)
                    .await
            }
        }
    }

    /// Why a conditional write was refused — read back AFTER the atomic attempt, so the classification can race but
    /// the write never could. An object left from an earlier incarnation of the bucket counts as absent, and an
    /// `If-None-Match` write over one replaces it under the same compare-and-set.
    async fn classify_refusal(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: &NewObject,
        condition: &WriteCondition,
    ) -> Result<Written> {
        let Snapshot {
            bucket: current,
            object: existing,
        } = self.repository.read(bucket, key).await?;
        if current != Some(incarnation) {
            return Ok(Written::NoSuchBucket);
        }
        let stale = existing
            .as_ref()
            .filter(|(record, _)| *record != incarnation)
            .map(|(record, _)| *record);
        let live = existing.is_some() && stale.is_none();
        match (condition, live, stale) {
            (WriteCondition::IfMatch(_), false, _) => Ok(Written::NoSuchKey),
            (WriteCondition::IfNoneMatch, false, Some(stale)) => {
                match self
                    .repository
                    .write(bucket, key, incarnation, object, &Guard::Incarnation(stale))
                    .await?
                {
                    Wrote::Record(value) => Ok(Written::Committed(read(&value)?.1.modified)),
                    Wrote::Refused => Ok(Written::PreconditionFailed),
                }
            }
            _ => Ok(Written::PreconditionFailed),
        }
    }

    /// The object at `bucket/key`: `Ok(Err(()))` when the bucket does not exist, `Ok(Ok(None))` when the key holds
    /// no live object.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn get(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> Result<std::result::Result<Option<StoredObject>, ()>> {
        // A multipart object's parts are read in a second snapshot that also checks the key still holds the same
        // upload; an upload's parts never change, they only go when the object does — then the key is read again.
        for _ in 0..MULTIPART_READ_ATTEMPTS {
            let Snapshot {
                bucket: current,
                object: existing,
            } = self.repository.read(bucket, key).await?;
            let Some(incarnation) = current else {
                return Ok(Err(()));
            };
            let Some((_, mut object)) = existing.filter(|(record, _)| *record == incarnation)
            else {
                return Ok(Ok(None));
            };
            let Content::Parts(multipart) = &mut object.content else {
                return Ok(Ok(Some(object)));
            };
            if let Some(parts) = self
                .repository
                .parts_of(bucket, key, multipart.upload)
                .await?
            {
                multipart.parts = parts;
                return Ok(Ok(Some(object)));
            }
        }
        Err(Error::Contended)
    }

    /// Removes each of `keys`, a few at a time, one outcome per key in order; `None` when the bucket does not exist.
    /// A missing key is removed successfully, as in S3.
    ///
    /// # Errors
    /// The metadata store's refusal or outage while reading the bucket; a key's own failure is its outcome.
    pub async fn delete_many(
        &self,
        bucket: &BucketName,
        keys: &[ObjectKey],
    ) -> Result<Option<Vec<Result<()>>>> {
        use futures_util::StreamExt;
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(None);
        }
        // Each removal owns its inputs: a future borrowing from the iterator's closure argument is not provably
        // `Send` for every lifetime, which the server's handler future must be.
        let removals = keys.iter().cloned().map(|key| {
            let repository = self.repository.clone();
            let bucket = bucket.clone();
            async move { repository.remove(&bucket, &key).await }
        });
        let outcomes = futures_util::stream::iter(removals)
            .buffered(DELETE_OBJECTS_CONCURRENCY)
            .collect()
            .await;
        Ok(Some(outcomes))
    }

    /// Removes the object at `bucket/key`; deleting a missing key succeeds, as in S3.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn delete(&self, bucket: &BucketName, key: &ObjectKey) -> Result<Removed> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(Removed::NoSuchBucket);
        }
        self.repository.remove(bucket, key).await?;
        Ok(Removed::Done)
    }

    /// Removes the object at `bucket/key` only while its ETag is `etag` — the check and the delete are one step.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn delete_if_match(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        etag: &str,
    ) -> Result<RemovedIf> {
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(RemovedIf::NoSuchBucket);
        };
        if self
            .repository
            .remove_if(bucket, key, incarnation, etag)
            .await?
        {
            return Ok(RemovedIf::Done);
        }
        // Why it was refused, read back after the atomic attempt: the classification can race, the delete never could.
        let Snapshot {
            bucket: current,
            object: existing,
        } = self.repository.read(bucket, key).await?;
        if current != Some(incarnation) {
            return Ok(RemovedIf::NoSuchBucket);
        }
        match existing {
            Some((record, _)) if record == incarnation => Ok(RemovedIf::PreconditionFailed),
            _ => Ok(RemovedIf::NoSuchKey),
        }
    }
}
