//! Object rules: a write commits only into a live bucket's current incarnation, under its condition, as one
//! statement; a read sees an object only if it belongs to the bucket's current incarnation. A data file is queued for
//! reclamation before it exists and leaves the queue only in the transaction that commits an object pointing at it.

use tessari_s3_constants::{MULTIPART_READ_ATTEMPTS, VERSION_WRITE_ATTEMPTS};
use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::read;
use super::model::{Content, NewObject, StoredObject, WriteCondition, Written};
use super::repository::{Guard, ObjectRepository, Snapshot, Wrote};
use super::tessaridb::TessariObjects;
use std::sync::Arc;

use crate::data::DataFiles;
use crate::erasure::ErasureWrites;
use crate::{Error, Result};

#[path = "service_bytes.rs"]
mod bytes;
#[path = "service_convert.rs"]
mod convert;
#[path = "service_data.rs"]
mod data;
#[path = "service_delete.rs"]
mod delete;
#[path = "list.rs"]
mod list;
#[path = "service_replica.rs"]
mod replica;
#[path = "service_version_list.rs"]
mod version_list;
#[path = "service_version_read.rs"]
mod version_read;
#[path = "service_versions.rs"]
mod versions;

/// Object operations.
#[derive(Clone)]
pub struct ObjectService {
    repository: TessariObjects,
    data: DataFiles,
    erasure: Option<Arc<ErasureWrites>>,
}

impl ObjectService {
    /// The service over `repository`, with data files in `data`.
    #[must_use]
    pub(crate) const fn new(repository: TessariObjects, data: DataFiles) -> Self {
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
        if let (Content::Data(id), false) = (
            &object.content,
            matches!(outcome, Written::Committed { .. }),
        ) {
            // A refusal may be the reclaimer's, not the condition's: the file is then already gone.
            if !self.repository.claimable(*id).await? {
                return Ok(Written::DataReclaimed);
            }
        }
        if let (Content::Data(id), false) = (
            &object.content,
            matches!(outcome, Written::Committed { .. }),
        ) {
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
        for _ in 0..VERSION_WRITE_ATTEMPTS {
            let plan = versions::plan(&self.repository.head(bucket, key).await?, key, false)?;
            match self
                .repository
                .write(bucket, key, incarnation, object, &guard, &plan)
                .await?
            {
                Wrote::Record(value) => {
                    return Ok(Written::Committed {
                        modified: read(&value)?.1.modified,
                        version: plan.label,
                    });
                }
                // A refusal is the condition's, the content's (its file reclaimed, its upload ended), or another
                // writer of the key committed first; while the condition and the content both still hold it was the
                // last, and the write is planned again.
                Wrote::Refused
                    if self.still_holds(bucket, key, incarnation, &guard).await?
                        && self.still_takeable(&object.content).await? => {}
                Wrote::Refused => {
                    return self
                        .classify_refusal(bucket, key, incarnation, object, condition)
                        .await;
                }
            }
        }
        Err(Error::Contended)
    }

    /// Whether `guard` holds on the key as it is now.
    async fn still_holds(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        guard: &Guard,
    ) -> Result<bool> {
        let Snapshot { object, .. } = self.repository.read(bucket, key).await?;
        Ok(match guard {
            Guard::None => true,
            // A record left from an earlier incarnation is the classifier's case, not a collision.
            Guard::Absent => object.is_none(),
            Guard::Etag(etag) => {
                object.is_some_and(|(record, stored)| record == incarnation && stored.etag == *etag)
            }
            Guard::Incarnation(_) => false,
        })
    }

    /// Whether what a write points at can still be taken: a data file no reclaimer marked, an upload nobody ended.
    async fn still_takeable(&self, content: &Content) -> Result<bool> {
        match content {
            Content::Data(id) => self.repository.claimable(*id).await,
            Content::Parts(multipart) => self.repository.upload_open(multipart.upload).await,
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
                let plan = versions::plan(&self.repository.head(bucket, key).await?, key, false)?;
                match self
                    .repository
                    .write(
                        bucket,
                        key,
                        incarnation,
                        object,
                        &Guard::Incarnation(stale),
                        &plan,
                    )
                    .await?
                {
                    Wrote::Record(value) => Ok(Written::Committed {
                        modified: read(&value)?.1.modified,
                        version: plan.label,
                    }),
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
}
