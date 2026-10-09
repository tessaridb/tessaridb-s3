//! Deleting objects: each removal changes the key's versions as its bucket's versioning state asks (ADR-0010) and is
//! planned again while another writer of the key keeps committing first.

use tessari_s3_constants::{DELETE_OBJECTS_CONCURRENCY, VERSION_WRITE_ATTEMPTS};
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::model::{Removed, RemovedIf, VersionRemoved};
use super::super::repository::{Guard, ObjectRepository, Snapshot};
use super::super::version_id::parse;
use super::{ObjectService, versions};
use crate::{Error, Result};

impl ObjectService {
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
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(None);
        };
        // Each removal owns its inputs: a future borrowing from the iterator's closure argument is not provably
        // `Send` for every lifetime, which the server's handler future must be.
        let removals = keys.iter().cloned().map(|key| {
            let repository = self.repository.clone();
            let bucket = bucket.clone();
            async move {
                versions::remove_key(&repository, &bucket, &key, incarnation)
                    .await
                    .map(|_| ())
            }
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
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(Removed::NoSuchBucket);
        };
        let marker = versions::remove_key(&self.repository, bucket, key, incarnation).await?;
        Ok(Removed::Done { marker })
    }

    /// Permanently removes the version of `bucket/key` whose id is `version_id`; when it was the newest, the next newest
    /// becomes the current object. Read and removed again while the key keeps moving under it.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to another writer.
    pub async fn delete_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
    ) -> Result<VersionRemoved> {
        let Some(named) = parse(version_id) else {
            return Ok(VersionRemoved::Malformed);
        };
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(VersionRemoved::NoSuchBucket);
        }
        for _ in 0..VERSION_WRITE_ATTEMPTS {
            let Some(target) = self
                .repository
                .version_target(bucket, key, named, version_id)
                .await?
            else {
                return Ok(VersionRemoved::Absent);
            };
            if self.repository.remove_version(bucket, key, &target).await? {
                return Ok(VersionRemoved::Done {
                    marker: target.marker,
                });
            }
        }
        Err(Error::Contended)
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
        let guard = Guard::Etag(etag.to_owned());
        for _ in 0..VERSION_WRITE_ATTEMPTS {
            let plan = versions::plan(&self.repository.head(bucket, key).await?, true)?;
            if self
                .repository
                .remove_if(bucket, key, incarnation, etag, &plan)
                .await?
            {
                return Ok(RemovedIf::Done);
            }
            if !self.still_holds(bucket, key, incarnation, &guard).await? {
                break;
            }
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
            Some((record, stored)) if record == incarnation && stored.etag == etag => {
                Err(Error::Contended)
            }
            Some((record, _)) if record == incarnation => Ok(RemovedIf::PreconditionFailed),
            _ => Ok(RemovedIf::NoSuchKey),
        }
    }
}
