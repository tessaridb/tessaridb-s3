//! What multipart persistence must answer.

use std::future::Future;

use tessari_s3_types::{BucketName, ObjectKey, PartNumber, UploadId};

use super::model::{NewPart, NewUpload, StoredPart};
use crate::Result;

/// An upload record: what it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UploadRecord {
    pub(crate) bucket: String,
    pub(crate) key: String,
    pub(crate) incarnation: [u8; 16],
}

/// Multipart queries.
pub(crate) trait MultipartRepository: Send + Sync {
    /// The bucket's incarnation, or `None` when the bucket does not exist.
    fn incarnation(
        &self,
        bucket: &BucketName,
    ) -> impl Future<Output = Result<Option<[u8; 16]>>> + Send;
    /// Records a new upload for `bucket/key` under `incarnation`.
    fn create(
        &self,
        id: UploadId,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        upload: &NewUpload,
    ) -> impl Future<Output = Result<()>> + Send;
    /// The upload record, if it exists.
    fn upload(&self, id: UploadId) -> impl Future<Output = Result<Option<UploadRecord>>> + Send;
    /// Writes part `number` of upload `id` in ONE transaction that also requires the upload to still exist and takes
    /// the part's data off the reclamation queue; `false` when the store refused it (the upload is gone, or the data
    /// was claimed by a reclaimer).
    fn write_part(
        &self,
        id: UploadId,
        number: PartNumber,
        part: &NewPart,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// At most `limit` parts of upload `id` numbered above `after`, ascending; plus whether one more exists.
    fn parts(
        &self,
        id: UploadId,
        after: u16,
        limit: usize,
    ) -> impl Future<Output = Result<(Vec<StoredPart>, bool)>> + Send;
    /// Removes the upload and every part record of it, in one transaction; each part's data is queued by the
    /// `part_superseded` event.
    fn abort(&self, id: UploadId) -> impl Future<Output = Result<()>> + Send;
}
