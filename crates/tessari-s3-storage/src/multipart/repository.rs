//! What multipart persistence must answer.

use std::future::Future;

use tessari_s3_core::objects::upload_listing::UploadAnchor;
use tessari_s3_types::{BucketName, ObjectKey, PartNumber, Timestamp, UploadId};

use super::model::{NewPart, NewUpload, OpenUpload, StoredPart};
use crate::Result;

/// An upload record: what it is bound to and what its Create fixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UploadRecord {
    pub(crate) bucket: String,
    pub(crate) key: String,
    pub(crate) incarnation: [u8; 16],
    pub(crate) declared: NewUpload,
    /// The upload as the listing orders it; `None` for an upload recorded before the listing existed.
    pub(crate) opened: Option<OpenUpload>,
}

/// An open upload as the listing order holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingRow {
    pub(crate) bucket: String,
    pub(crate) key: String,
    pub(crate) incarnation: [u8; 16],
    pub(crate) upload: OpenUpload,
}

/// One read of the listing order, with the bucket's incarnation at the same snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingBatch {
    pub(crate) bucket: Option<[u8; 16]>,
    pub(crate) rows: Vec<PendingRow>,
    /// Whether the read filled its limit, so more rows may follow.
    pub(crate) full: bool,
}

/// A part as Complete reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartRow {
    pub(crate) number: u16,
    pub(crate) data: [u8; 16],
    pub(crate) record: tessari_s3_core::objects::multipart::PartRecord,
}

/// Multipart queries.
pub(crate) trait MultipartRepository: Send + Sync {
    /// The bucket's incarnation, or `None` when the bucket does not exist.
    fn incarnation(
        &self,
        bucket: &BucketName,
    ) -> impl Future<Output = Result<Option<[u8; 16]>>> + Send;
    /// Records a new upload for `bucket/key` under `incarnation`, started at `initiated`, and its place in the
    /// listing order, in one transaction.
    fn create(
        &self,
        id: UploadId,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        initiated: Timestamp,
        upload: &NewUpload,
    ) -> impl Future<Output = Result<()>> + Send;
    /// The listing-order position a walk from `anchor` starts after.
    fn position_of(&self, bucket: &BucketName, anchor: &UploadAnchor<OpenUpload>) -> String;
    /// At most `limit` open uploads after listing position `after`, in listing order.
    fn pending(
        &self,
        bucket: &BucketName,
        after: &str,
        limit: usize,
    ) -> impl Future<Output = Result<PendingBatch>> + Send;
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
    /// Every part of upload `id`.
    fn all_parts(&self, id: UploadId) -> impl Future<Output = Result<Vec<PartRow>>> + Send;
    /// Removes the upload and every part record of it, in one transaction; each part's data is queued by the
    /// `part_superseded` event.
    fn abort(&self, id: UploadId) -> impl Future<Output = Result<()>> + Send;
}
