//! Reads naming a version, and the delete marker a current read answers with (S3 ref 08 M4).

use tessari_s3_constants::MULTIPART_READ_ATTEMPTS;
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::model::{Content, VersionRead};
use super::super::repository::ObjectRepository;
use super::super::tessaridb::Found;
use super::super::version_id::parse;
use super::ObjectService;
use crate::{Error, Result};

impl ObjectService {
    /// The version of `bucket/key` whose id is `version_id`. The bucket's incarnation is read first and a version of
    /// another incarnation is no version of this bucket; a multipart version's parts are read, then the version is
    /// confirmed to still name them.
    ///
    /// # Errors
    /// The metadata store's refusal or outage; [`Error::Contended`] when the version kept going while being read.
    pub async fn get_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: &str,
    ) -> Result<VersionRead> {
        let Some(named) = parse(version_id) else {
            return Ok(VersionRead::Malformed);
        };
        let Some(current) = self.repository.incarnation(bucket).await? else {
            return Ok(VersionRead::NoSuchBucket);
        };
        for _ in 0..MULTIPART_READ_ATTEMPTS {
            match self
                .repository
                .version_found(bucket, key, named, version_id)
                .await?
            {
                Some(Found::Marker {
                    incarnation,
                    modified,
                }) if incarnation == current => return Ok(VersionRead::Marker { modified }),
                Some(Found::Object {
                    incarnation,
                    mut object,
                    at,
                }) if incarnation == current => {
                    let Content::Parts(multipart) = &mut object.content else {
                        return Ok(VersionRead::Object(*object));
                    };
                    if let Some(parts) = self.repository.version_parts(at, multipart.upload).await?
                    {
                        multipart.parts = parts;
                        return Ok(VersionRead::Object(*object));
                    }
                }
                _ => return Ok(VersionRead::Absent),
            }
        }
        Err(Error::Contended)
    }

    /// The id of the delete marker that is `bucket/key`'s newest version, when it is one — what a read of the key
    /// that finds no object answers with.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn current_marker(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> Result<Option<String>> {
        let Some(current) = self.repository.incarnation(bucket).await? else {
            return Ok(None);
        };
        Ok(self
            .repository
            .newest_marker(bucket, key)
            .await?
            .filter(|(incarnation, _)| *incarnation == current)
            .map(|(_, id)| id))
    }
}
