//! Bucket rules: creating, finding, listing and removing buckets.

use tessari_s3_types::BucketName;

use super::model::Bucket;
use super::repository::{BucketRepository, Inserted};
use super::tessaridb::TessariBuckets;
use crate::Result;

/// What creating a bucket ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    /// The bucket now exists.
    Created(Bucket),
    /// It already existed; one owner holds every bucket, so it is the caller's.
    AlreadyOwned,
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

    /// Creates `name` in `region`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(&self, name: &BucketName, region: &str) -> Result<Created> {
        match self.repository.insert(name, region).await? {
            Inserted::Created(entity) => Ok(Created::Created(Bucket::try_from(entity)?)),
            Inserted::Exists => Ok(Created::AlreadyOwned),
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

    /// Every bucket, in byte order of their names.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn list(&self) -> Result<Vec<Bucket>> {
        let mut buckets: Vec<Bucket> = self
            .repository
            .list()
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
