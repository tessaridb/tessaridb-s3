//! A version's tags through the service: read them, replace them, remove them.

use std::collections::BTreeMap;

use tessari_s3_types::{BucketName, ObjectKey};

use super::super::model_tags::TagsOutcome;
use super::super::repository::ObjectRepository;
use super::ObjectService;
use crate::Result;

impl ObjectService {
    /// The tags of the version `version` names, or of the key's current one.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn version_tags(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
    ) -> Result<TagsOutcome> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(TagsOutcome::NoSuchBucket);
        }
        self.repository.version_tags(bucket, key, version).await
    }

    /// Replaces the version's tags; an empty set removes them.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`crate::Error::Contended`] when every attempt lost to a writer of the
    /// key.
    pub async fn set_version_tags(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        tags: &BTreeMap<String, String>,
    ) -> Result<TagsOutcome> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(TagsOutcome::NoSuchBucket);
        }
        self.repository
            .set_version_tags(bucket, key, version, tags)
            .await
    }
}
