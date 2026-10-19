//! Deleting objects: each removal changes the key's versions as its bucket's versioning state asks (ADR-0010) and is
//! planned again while another writer of the key keeps committing first.

use tessari_s3_constants::{DELETE_OBJECTS_CONCURRENCY, VERSION_WRITE_ATTEMPTS};
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::model::{Deleted, Removed, RemovedIf, VersionRemoved};
use super::super::repository::{Guard, ObjectRepository, Removal, Snapshot};
use super::super::tessaridb::TessariObjects;
use super::super::version_id::parse;
use super::{ObjectService, versions};
use crate::{Error, Result};

/// Removes the version of `bucket/key` whose id is `version_id`, read and removed again while the key keeps moving.
async fn remove_named(
    repository: &TessariObjects,
    bucket: &BucketName,
    key: &ObjectKey,
    version_id: &str,
    removal: Removal,
    bypass: bool,
) -> Result<VersionRemoved> {
    let Some(named) = parse(version_id) else {
        return Ok(VersionRemoved::Malformed);
    };
    for _ in 0..VERSION_WRITE_ATTEMPTS {
        let Some(target) = repository
            .version_target(bucket, key, named, version_id)
            .await?
        else {
            return Ok(VersionRemoved::Absent);
        };
        if target.kept.keeps(bypass) {
            return Ok(VersionRemoved::Locked);
        }
        if repository
            .remove_version(bucket, key, &target, removal, bypass)
            .await?
        {
            return Ok(VersionRemoved::Done {
                marker: target.marker,
            });
        }
    }
    Err(Error::Contended)
}

impl ObjectService {
    /// Removes each entry — the current object of a key, or the version it names (`bypass` lets a GOVERNANCE retention
    /// go; the caller has checked who may ask) — a few keys at a time, one outcome per entry in order; `None` when the
    /// bucket does not exist. A missing key or version is removed successfully, as in S3.
    ///
    /// The entries naming one key are removed one after another: run side by side they would fence each other (each
    /// removal guards the version its plan read) and the request would contend with itself.
    ///
    /// # Errors
    /// The metadata store's refusal or outage while reading the bucket; a key's own failure is its outcome.
    pub async fn delete_many(
        &self,
        bucket: &BucketName,
        entries: &[(ObjectKey, Option<String>)],
        bypass: bool,
    ) -> Result<Option<Vec<Result<Deleted>>>> {
        use futures_util::StreamExt;
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(None);
        };
        // Each key's entries with their positions, in the order the keys first appear.
        let mut groups: Vec<Vec<(usize, Option<String>)>> = Vec::new();
        let mut keys: Vec<ObjectKey> = Vec::new();
        let mut slots: std::collections::HashMap<&ObjectKey, usize> =
            std::collections::HashMap::new();
        for (position, (key, version)) in entries.iter().enumerate() {
            let slot = *slots.entry(key).or_insert_with(|| {
                groups.push(Vec::new());
                keys.push(key.clone());
                groups.len().saturating_sub(1)
            });
            if let Some(group) = groups.get_mut(slot) {
                group.push((position, version.clone()));
            }
        }
        // Each removal owns its inputs: a future borrowing from the iterator's closure argument is not provably
        // `Send` for every lifetime, which the server's handler future must be.
        let removals = keys.into_iter().zip(groups).map(|(key, group)| {
            let repository = self.repository.clone();
            let bucket = bucket.clone();
            async move {
                let mut done = Vec::with_capacity(group.len());
                for (position, version) in group {
                    let outcome = match version {
                        Some(version) => remove_named(
                            &repository,
                            &bucket,
                            &key,
                            &version,
                            Removal::Request,
                            bypass,
                        )
                        .await
                        .map(Deleted::Version),
                        None => versions::remove_key(&repository, &bucket, &key, incarnation)
                            .await
                            .map(|marker| Deleted::Current { marker }),
                    };
                    done.push((position, outcome));
                }
                done
            }
        });
        let mut outcomes: Vec<(usize, Result<Deleted>)> = futures_util::stream::iter(removals)
            .buffer_unordered(DELETE_OBJECTS_CONCURRENCY)
            .flat_map(futures_util::stream::iter)
            .collect()
            .await;
        outcomes.sort_unstable_by_key(|(position, _)| *position);
        Ok(Some(
            outcomes.into_iter().map(|(_, outcome)| outcome).collect(),
        ))
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
    /// becomes the current object. Read and removed again while the key keeps moving under it. A version Object Lock
    /// keeps is [`VersionRemoved::Locked`]; `bypass` lets a GOVERNANCE retention go (the caller has checked who may).
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to another writer.
    pub async fn delete_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
        bypass: bool,
    ) -> Result<VersionRemoved> {
        self.remove_version_for(bucket, key, version_id, Removal::Request, bypass)
            .await
    }

    /// Permanently removes the version of `bucket/key` whose id is `version_id` because the bucket's lifecycle rules
    /// expire it, as [`Self::delete_version`] does for a request.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to another writer.
    pub async fn expire_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
    ) -> Result<VersionRemoved> {
        self.remove_version_for(bucket, key, version_id, Removal::Lifecycle, false)
            .await
    }

    async fn remove_version_for(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
        removal: Removal,
        bypass: bool,
    ) -> Result<VersionRemoved> {
        if parse(version_id).is_none() {
            return Ok(VersionRemoved::Malformed);
        }
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(VersionRemoved::NoSuchBucket);
        }
        remove_named(&self.repository, bucket, key, version_id, removal, bypass).await
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
            let plan = versions::plan(&self.repository.head(bucket, key).await?, key, true)?;
            if self
                .repository
                .remove_if(bucket, key, incarnation, etag, None, &plan)
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

    /// Expires the current object at `bucket/key` (S3 ref 08 M7) only while it is still the one written at `modified`
    /// with ETag `etag` — the check and the delete are one step — leaving a delete marker where the bucket keeps
    /// versions. `false` when the key moved on or is gone: nothing was removed.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when every attempt lost to a version writer.
    pub async fn expire_current(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        etag: &str,
        modified: tessari_s3_types::Timestamp,
    ) -> Result<bool> {
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(false);
        };
        for _ in 0..VERSION_WRITE_ATTEMPTS {
            let mut plan = versions::plan(&self.repository.head(bucket, key).await?, key, true)?;
            plan.removal = Removal::Lifecycle;
            // A delete marker a lifecycle rule adds is not replicated (S3 ref 11 M4: only a request's marker is).
            plan.replicate = None;
            if self
                .repository
                .remove_if(bucket, key, incarnation, etag, Some(modified), &plan)
                .await?
            {
                return Ok(true);
            }
            // Refused: try again only while the same object is current — a newer write keeps it from expiring.
            let still =
                self.repository
                    .read(bucket, key)
                    .await?
                    .object
                    .is_some_and(|(record, stored)| {
                        record == incarnation && stored.etag == etag && stored.modified == modified
                    });
            if !still {
                return Ok(false);
            }
        }
        Err(Error::Contended)
    }
}
