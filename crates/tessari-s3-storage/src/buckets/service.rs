//! Bucket rules: creating, finding, listing and removing buckets.

use tessari_s3_core::authz::{SpaceName, Visible};
use tessari_s3_core::quota::Quota;
use tessari_s3_core::versioning::Versioning;
use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::BucketName;

use super::model::{Acknowledgement, Bucket, LockSet};
use super::repository::{BucketRepository, Inserted};
use super::tessaridb::TessariBuckets;
use crate::Result;

/// What creating a bucket ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    /// The bucket now exists.
    Created(Box<Bucket>),
    /// A bucket of that name already exists, in some space: bucket names are global.
    Exists,
}

/// What setting a bucket's acknowledgement ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcknowledgementSet {
    /// The bucket's writes are acknowledged as asked from its next write on.
    Set,
    /// There was no such bucket.
    NoSuchBucket,
    /// The metadata store refused a region-majority acknowledgement here — in its own words (a store whose nodes name
    /// no region has no region majority to count); nothing was changed.
    Refused(String),
}

/// What deleting a bucket ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deleted {
    /// The bucket is gone.
    Deleted,
    /// There was no such bucket.
    NoSuchBucket,
    /// The bucket still holds objects and was kept.
    NotEmpty,
}

/// Bucket operations.
#[derive(Clone)]
pub struct BucketService {
    repository: TessariBuckets,
}

impl BucketService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariBuckets) -> Self {
        Self { repository }
    }

    /// Creates `name` in `region`, owned by `space` and made by `creator` (`None` for the root credential); with
    /// `object_lock` it has Object Lock on and versioning enabled from its first instant.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(
        &self,
        name: &BucketName,
        region: &str,
        space: &SpaceName,
        creator: Option<&str>,
        object_lock: bool,
    ) -> Result<Created> {
        match self
            .repository
            .insert(name, region, space, creator, object_lock)
            .await?
        {
            Inserted::Created(entity) => Ok(Created::Created(Box::new(Bucket::try_from(*entity)?))),
            Inserted::Exists => Ok(Created::Exists),
        }
    }

    /// The bucket called `name`, if it exists.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn get(&self, name: &BucketName) -> Result<Option<Bucket>> {
        self.repository
            .get(name)
            .await?
            .map(Bucket::try_from)
            .transpose()
    }

    /// Sets `name`'s limits, replacing both; `false` when there is no such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_quota(&self, name: &BucketName, quota: Quota) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        let stored = |limit: Option<u64>| {
            limit
                .map(i64::try_from)
                .transpose()
                .map_err(|_| crate::Error::Malformed {
                    record: "bucket",
                    reason: "a quota limit above what the store holds",
                })
        };
        self.repository
            .set_limits(name, stored(quota.max_bytes)?, stored(quota.max_objects)?)
            .await?;
        Ok(true)
    }

    /// Sets who must hold `name`'s object writes before they are acknowledged. A region majority is first proved
    /// by a write this region's database acknowledges that way, so a bucket is never set to a level every one of its
    /// writes would then be refused.
    ///
    /// # Errors
    /// The metadata store's refusal (other than of the level itself) or outage.
    pub async fn set_acknowledgement(
        &self,
        name: &BucketName,
        acknowledgement: Acknowledgement,
    ) -> Result<AcknowledgementSet> {
        if self.repository.get(name).await?.is_none() {
            return Ok(AcknowledgementSet::NoSuchBucket);
        }
        let local = acknowledgement == Acknowledgement::RegionMajority;
        if local && let Some(refusal) = self.repository.probe_local().await? {
            return Ok(AcknowledgementSet::Refused(refusal));
        }
        self.repository.set_acknowledge(name, local).await?;
        Ok(AcknowledgementSet::Set)
    }

    /// Sets `name`'s versioning state, answering `false` when there is no such bucket. A state is always one of the
    /// two versioned ones, so a versioned bucket can never be returned to never-versioned.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_versioning(&self, name: &BucketName, state: Versioning) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        self.repository.set_versioning(name, state.as_str()).await?;
        Ok(true)
    }

    /// Sets `name`'s lifecycle rules (already validated), or removes them with `None`; `false` when there is no such
    /// bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_lifecycle(
        &self,
        name: &BucketName,
        rules: Option<&[tessari_s3_core::lifecycle::Rule]>,
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        self.repository
            .set_lifecycle(name, rules.map(super::lifecycle::to_value))
            .await?;
        Ok(true)
    }

    /// Turns Object Lock on for `name`, or replaces its default retention; the bucket's versioning must be enabled.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_object_lock(
        &self,
        name: &BucketName,
        configuration: &tessari_s3_core::object_lock::Configuration,
    ) -> Result<LockSet> {
        let Some(bucket) = self.repository.get(name).await? else {
            return Ok(LockSet::NoSuchBucket);
        };
        if bucket.versioning.as_deref() != Some("Enabled") {
            return Ok(LockSet::NotVersioned);
        }
        if self
            .repository
            .set_object_lock(name, super::object_lock::to_value(configuration))
            .await?
        {
            Ok(LockSet::Done)
        } else {
            Ok(LockSet::NotVersioned)
        }
    }

    /// Replaces `name`'s tags (already validated); an empty set removes them. `false` when there is no such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_tags(
        &self,
        name: &BucketName,
        tags: &std::collections::BTreeMap<String, String>,
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        let value = (!tags.is_empty()).then(|| {
            Value::Object(
                tags.iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect(),
            )
        });
        self.repository.set_tags(name, value).await?;
        Ok(true)
    }

    /// Sets `name`'s replication configuration (already validated), or removes it with `None`; `false` when there is
    /// no such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_replication(
        &self,
        name: &BucketName,
        configuration: Option<&tessari_s3_core::replication::Configuration>,
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        self.repository
            .set_replication(name, configuration.map(super::replication::to_value))
            .await?;
        Ok(true)
    }

    /// Sets `name`'s notification rules (already validated); no rules turns notifications off. `false` when there is
    /// no such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_notification(
        &self,
        name: &BucketName,
        rules: &[tessari_s3_core::notification::Rule],
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        let configuration = (!rules.is_empty()).then(|| super::notification::to_value(rules));
        self.repository
            .set_notification(name, configuration)
            .await?;
        Ok(true)
    }

    /// Sets `name`'s media rules (already validated); no rules turns media processing off. `false` when there is no
    /// such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_media(
        &self,
        name: &BucketName,
        rules: &[tessari_s3_core::media::Rule],
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        let stored = (!rules.is_empty()).then(|| super::media::to_value(rules));
        self.repository.set_media(name, stored).await?;
        Ok(true)
    }

    /// The buckets `visible` covers, in byte order of their names.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn list(&self, visible: &Visible) -> Result<Vec<Bucket>> {
        let mut buckets: Vec<Bucket> = self
            .repository
            .list(visible)
            .await?
            .into_iter()
            .map(Bucket::try_from)
            .collect::<Result<_>>()?;
        buckets.sort_by(|a, b| a.name.as_str().as_bytes().cmp(b.name.as_str().as_bytes()));
        Ok(buckets)
    }

    /// Deletes `name` when it holds no objects.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn delete(&self, name: &BucketName) -> Result<Deleted> {
        if self.repository.get(name).await?.is_none() {
            return Ok(Deleted::NoSuchBucket);
        }
        if self.repository.holds_objects(name).await? {
            return Ok(Deleted::NotEmpty);
        }
        match self.repository.remove(name).await? {
            Some(_) => Ok(Deleted::Deleted),
            None => Ok(Deleted::NoSuchBucket),
        }
    }
}
