//! Objects the server reads and writes whole, for its own work: an original a media worker renders from, held in
//! memory under a bound, and a rendition it writes back as an ordinary object.

use std::collections::BTreeMap;

use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_types::{BucketName, ObjectKey};

use super::ObjectService;
use crate::Result;
use crate::objects::{Content, NewObject, StoredObject, VersionRead, WriteCondition, Written};

impl ObjectService {
    /// The object `version_id` names at `bucket/key`, or the current one when none is named; none when it is gone, a
    /// delete marker, or its bucket is.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn stored(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version_id: Option<&str>,
    ) -> Result<Option<StoredObject>> {
        Ok(match version_id {
            Some(version) => match self.get_version(bucket, key, version).await? {
                VersionRead::Object(object) => Some(object),
                _ => None,
            },
            None => self.get(bucket, key).await?.ok().flatten(),
        })
    }

    /// Every byte of `object`, or none when it is larger than `max` bytes; a multipart object is read part by part.
    ///
    /// # Errors
    /// A data file's or the shards' failure.
    pub async fn read_whole(&self, object: &StoredObject, max: u64) -> Result<Option<Vec<u8>>> {
        if object.size > max {
            return Ok(None);
        }
        let pieces = match &object.content {
            Content::Data(id) => vec![(*id, object.size)],
            Content::Parts(multipart) => multipart
                .parts
                .iter()
                .map(|part| (part.data, part.size))
                .collect(),
        };
        let mut bytes = Vec::with_capacity(usize::try_from(object.size).unwrap_or(0));
        for (id, size) in pieces {
            let reader = self.open(id, size).await?;
            let block = u64::from(reader.block_size()).max(1);
            for index in 0..size.div_ceil(block) {
                bytes.extend_from_slice(&reader.read_block(index).await?);
            }
        }
        Ok(Some(bytes))
    }

    /// Writes `bytes` at `bucket/key` as an object of `content_type`, over whatever is there, with an MD5 ETag and the
    /// default checksum as a PUT of the same bytes would have. A write that does not commit releases its file.
    ///
    /// # Errors
    /// The drive's failure, or the metadata store's refusal or outage.
    pub async fn put_bytes(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<Written> {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut upload = self.upload(length).await?;
        let id = upload.id();
        let uploaded = match upload.append(bytes).await {
            Ok(()) => upload.finish().await,
            Err(error) => Err(error),
        };
        let uploaded = match uploaded {
            Ok(uploaded) => uploaded,
            Err(error) => {
                self.release_logged(id).await;
                return Err(error);
            }
        };
        let algorithm = ChecksumAlgorithm::Crc64Nvme;
        let object = NewObject {
            size: uploaded.size,
            etag: format!("\"{}\"", uploaded.digests.md5_hex),
            headers: BTreeMap::from([("content-type".to_owned(), content_type.to_owned())]),
            metadata: BTreeMap::new(),
            checksums: BTreeMap::from([(
                algorithm.name().to_owned(),
                uploaded.digests.value(algorithm).to_owned(),
            )]),
            content: Content::Data(uploaded.id),
            copied: false,
        };
        self.put(bucket, key, &object, &WriteCondition::None).await
    }
}
