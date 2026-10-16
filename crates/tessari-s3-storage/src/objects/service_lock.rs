//! A version's Object Lock through the service: read it, set its retention, turn its legal hold on or off. Whether the
//! bucket has Object Lock at all is the caller's question (a bucket without it refuses these requests).

use tessari_s3_core::object_lock::Mode;
use tessari_s3_types::{BucketName, ObjectKey, Timestamp};

use super::super::model_lock::LockOutcome;
use super::super::repository::ObjectRepository;
use super::ObjectService;
use crate::Result;

impl ObjectService {
    /// The Object Lock of the version `version` names, or of the key's current one.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn version_lock(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
    ) -> Result<LockOutcome> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(LockOutcome::NoSuchBucket);
        }
        self.repository.version_lock(bucket, key, version).await
    }

    /// Sets the version's retention, or removes it with `None`; shortening or removing one that still keeps the
    /// version is [`LockOutcome::Locked`] unless it is GOVERNANCE and `bypass` (whose caller checked who may ask).
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_retention(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        retention: Option<(Mode, Timestamp)>,
        bypass: bool,
    ) -> Result<LockOutcome> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(LockOutcome::NoSuchBucket);
        }
        self.repository
            .set_retention(bucket, key, version, retention, bypass)
            .await
    }

    /// Turns the version's legal hold on or off.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_legal_hold(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        on: bool,
    ) -> Result<LockOutcome> {
        if self.repository.incarnation(bucket).await?.is_none() {
            return Ok(LockOutcome::NoSuchBucket);
        }
        self.repository
            .set_legal_hold(bucket, key, version, on)
            .await
    }
}
