//! ListMultipartUploads: the core upload lister fed batches of open uploads read in listing order. Only the bucket's
//! current incarnation is shown, and the walk stops at the first upload of another bucket.

use tessari_s3_constants::LIST_UPLOADS_MAX;
use tessari_s3_core::objects::upload_listing::{Marker, UploadLister, UploadSpec};
use tessari_s3_types::{BucketName, UploadId};

use super::model::{OpenUpload, UploadsListed};
use super::repository::{MultipartRepository, UploadRecord};
use super::service::MultipartService;
use crate::Result;

impl MultipartService {
    /// One ListMultipartUploads page of `bucket` as `spec` asks for it, resumed after `upload_marker` when the
    /// request names one.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn list_uploads(
        &self,
        bucket: &BucketName,
        spec: UploadSpec,
        upload_marker: Option<UploadId>,
    ) -> Result<UploadsListed> {
        let marker = self
            .marker(bucket, spec.key_marker.as_deref(), upload_marker)
            .await?;
        // One more than the page so the peek that decides IsTruncated usually costs no second read.
        let batch_size = spec
            .max_uploads
            .saturating_add(1)
            .clamp(1, LIST_UPLOADS_MAX);
        let mut lister = UploadLister::new(spec, marker);
        let mut current = None;
        // Where to continue when a whole batch held nothing visible (uploads of an earlier incarnation): the lister
        // saw nothing, so its anchor did not move, and this cursor does.
        let mut past_invisible: Option<String> = None;
        loop {
            let after = match (past_invisible.take(), lister.anchor()) {
                (Some(position), _) => position,
                (None, None) => break,
                (None, Some(anchor)) => self.repository.position_of(bucket, anchor),
            };
            let batch = self.repository.pending(bucket, &after, batch_size).await?;
            let Some(now) = batch.bucket else {
                return Ok(UploadsListed::NoSuchBucket);
            };
            // The incarnation the listing started in holds for every page of it.
            let incarnation = *current.get_or_insert(now);
            let mut more = batch.full;
            let mut visible = Vec::with_capacity(batch.rows.len());
            let mut last = None;
            for row in batch.rows {
                if row.bucket != bucket.as_str() {
                    more = false;
                    break;
                }
                if row.incarnation == incarnation {
                    visible.push((row.key, row.upload));
                } else {
                    last = Some(row.upload.position);
                }
            }
            if visible.is_empty() && more {
                past_invisible = last;
                if past_invisible.is_none() {
                    break;
                }
                continue;
            }
            lister.feed(visible, more);
        }
        Ok(UploadsListed::Page(lister.finish()))
    }

    /// The upload-id-marker resolved against the key-marker: S3 ignores it without a key-marker, and an id that names
    /// no open upload of that key in this bucket's current incarnation is gone.
    async fn marker(
        &self,
        bucket: &BucketName,
        key_marker: Option<&str>,
        upload_marker: Option<UploadId>,
    ) -> Result<Marker<OpenUpload>> {
        let (Some(key), Some(id)) = (key_marker, upload_marker) else {
            return Ok(Marker::None);
        };
        let Some(UploadRecord {
            bucket: upload_bucket,
            key: upload_key,
            incarnation,
            opened: Some(opened),
            ..
        }) = self.repository.upload(id).await?
        else {
            return Ok(Marker::Gone);
        };
        let current = self.repository.incarnation(bucket).await?;
        Ok(
            if upload_bucket == bucket.as_str() && upload_key == key && current == Some(incarnation)
            {
                Marker::Found(opened)
            } else {
                Marker::Gone
            },
        )
    }
}
