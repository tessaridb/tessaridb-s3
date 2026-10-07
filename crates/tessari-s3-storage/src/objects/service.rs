//! Object rules: a write commits only into a live bucket's current incarnation, under its condition, as one
//! statement; a read sees an object only if it belongs to the bucket's current incarnation.

use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::read;
use super::model::{NewObject, Removed, StoredObject, WriteCondition, Written};
use super::repository::{Guard, ObjectRepository, Snapshot, Wrote};
use super::tessaridb::TessariObjects;
use crate::Result;

/// Object operations.
#[derive(Clone)]
pub struct ObjectService {
    repository: TessariObjects,
}

impl ObjectService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariObjects) -> Self {
        Self { repository }
    }

    /// Commits `object` at `bucket/key` under `condition`.
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
        let Snapshot {
            bucket: current,
            object: existing,
        } = self.repository.read(bucket, key).await?;
        let Some(incarnation) = current else {
            return Ok(Err(()));
        };
        Ok(Ok(existing
            .filter(|(record, _)| *record == incarnation)
            .map(|(_, object)| object)))
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
}
