//! Setting a bucket's default encryption.

use tessari_s3_types::BucketName;

use super::model::BucketEncryption;
use super::repository::BucketRepository;
use super::service::BucketService;
use crate::Result;

impl BucketService {
    /// Sets `name`'s default encryption (already validated and its key held), or removes it with `None`; `false` when
    /// there is no such bucket.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_encryption(
        &self,
        name: &BucketName,
        encryption: Option<&BucketEncryption>,
    ) -> Result<bool> {
        if self.repository.get(name).await?.is_none() {
            return Ok(false);
        }
        self.repository
            .set_encryption(name, encryption.map(super::encryption::to_value))
            .await?;
        Ok(true)
    }
}
