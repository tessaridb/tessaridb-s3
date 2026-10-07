//! Multipart rules: an upload belongs to one key of one incarnation of a bucket, a part is accepted only while its
//! upload exists, and a part's data file is queued for reclamation before it exists, exactly as an object's is.

use tessari_s3_types::{BucketName, ObjectKey, PartNumber, UploadId};

use super::model::{Created, NewPart, NewUpload, PartWritten, PartsPage};
use super::repository::{MultipartRepository, UploadRecord};
use super::tessaridb::TessariMultipart;
use crate::objects::ObjectService;
use crate::{Error, Result};

/// Multipart operations.
#[derive(Clone)]
pub struct MultipartService {
    repository: TessariMultipart,
    objects: ObjectService,
}

impl MultipartService {
    /// The service over `repository`; data files are written and released through `objects`.
    #[must_use]
    pub(crate) const fn new(repository: TessariMultipart, objects: ObjectService) -> Self {
        Self {
            repository,
            objects,
        }
    }

    /// Starts an upload for `bucket/key` with a fresh random id.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload: &NewUpload,
    ) -> Result<Created> {
        let Some(incarnation) = self.repository.incarnation(bucket).await? else {
            return Ok(Created::NoSuchBucket);
        };
        // 128 bits from the operating system's random source: an upload id is the only thing standing between a
        // caller and another caller's upload.
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| Error::Randomness)?;
        let id = UploadId::from_bytes(bytes);
        self.repository
            .create(id, bucket, key, incarnation, upload)
            .await?;
        Ok(Created::Created(id))
    }

    /// Whether `id` is an upload of `bucket/key` in the bucket's current incarnation.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn exists(&self, bucket: &BucketName, key: &ObjectKey, id: UploadId) -> Result<bool> {
        let Some(UploadRecord {
            bucket: upload_bucket,
            key: upload_key,
            incarnation,
        }) = self.repository.upload(id).await?
        else {
            return Ok(false);
        };
        let current = self.repository.incarnation(bucket).await?;
        Ok(upload_bucket == bucket.as_str()
            && upload_key == key.as_str()
            && current == Some(incarnation))
    }

    /// Commits part `number` of upload `id`, whose bytes are in `part.data`. A part the store refuses releases its
    /// data file.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn write_part(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        id: UploadId,
        number: PartNumber,
        part: &NewPart,
    ) -> Result<PartWritten> {
        if !self.exists(bucket, key, id).await? {
            self.release_logged(part.data).await;
            return Ok(PartWritten::NoSuchUpload);
        }
        if self.repository.write_part(id, number, part).await? {
            return Ok(PartWritten::Committed);
        }
        // Refused: the upload went away in between, or a reclaimer claimed the data first.
        if self.repository.upload(id).await?.is_none() {
            self.release_logged(part.data).await;
            return Ok(PartWritten::NoSuchUpload);
        }
        if !self.objects.claimable(part.data).await? {
            return Ok(PartWritten::DataReclaimed);
        }
        // Neither explains the refusal: the outcome is unknown, so the data stays queued for the reclaimer.
        Err(Error::Malformed {
            record: "part",
            reason: "a part commit was refused with its upload and data in place",
        })
    }

    async fn release_logged(&self, data: [u8; 16]) {
        if let Err(error) = self.objects.release(data).await {
            let data = uuid::Uuid::from_bytes(data);
            tracing::warn!(%data, error = %error, "part data left queued for the reclaimer");
        }
    }

    /// One ListParts page of upload `id`: parts numbered above `after`, at most `limit`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn parts(&self, id: UploadId, after: u16, limit: usize) -> Result<PartsPage> {
        let (parts, truncated) = self.repository.parts(id, after, limit).await?;
        Ok(PartsPage { parts, truncated })
    }

    /// Ends upload `id`: its record and every part go, and every part's data is queued for reclamation.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn abort(&self, id: UploadId) -> Result<()> {
        self.repository.abort(id).await
    }
}
