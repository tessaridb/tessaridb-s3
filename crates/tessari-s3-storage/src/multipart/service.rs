//! Multipart rules: an upload belongs to one key of one incarnation of a bucket, a part is accepted only while its
//! upload exists, and a part's data file is queued for reclamation before it exists, exactly as an object's is.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_core::objects::multipart::{ObjectChecksum, PartRecord, assemble};
use tessari_s3_types::{BucketName, ObjectKey, PartNumber, Timestamp, UploadId};

use super::model::{Completed, Completion, Created, NewPart, NewUpload, PartWritten, PartsPage};
use super::repository::{MultipartRepository, UploadRecord};
use super::tessaridb::TessariMultipart;
use crate::objects::{Content, Multipart, NewObject, ObjectService, Part, Written};
use crate::{Error, Result};

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "upload",
        reason,
    }
}

fn clock() -> Error {
    Error::Malformed {
        record: "clock",
        reason: "the system clock is outside the representable range",
    }
}

/// Multipart operations.
#[derive(Clone)]
pub struct MultipartService {
    pub(super) repository: TessariMultipart,
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
        // One instant for the record and its place in the listing, so the time shown and the order agree.
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| clock())?;
        let initiated = Timestamp {
            seconds: i64::try_from(since.as_secs()).map_err(|_| clock())?,
            nanos: since.subsec_nanos(),
        };
        self.repository
            .create(id, bucket, key, incarnation, initiated, upload)
            .await?;
        Ok(Created::Created(id))
    }

    /// What Create fixed for upload `id`, when it is an upload of `bucket/key` in the bucket's current incarnation.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn find(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        id: UploadId,
    ) -> Result<Option<NewUpload>> {
        let Some(UploadRecord {
            bucket: upload_bucket,
            key: upload_key,
            incarnation,
            declared,
            ..
        }) = self.repository.upload(id).await?
        else {
            return Ok(None);
        };
        let current = self.repository.incarnation(bucket).await?;
        Ok((upload_bucket == bucket.as_str()
            && upload_key == key.as_str()
            && current == Some(incarnation))
        .then_some(declared))
    }

    /// Whether `id` is an upload of `bucket/key` in the bucket's current incarnation.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn exists(&self, bucket: &BucketName, key: &ObjectKey, id: UploadId) -> Result<bool> {
        Ok(self.find(bucket, key, id).await?.is_some())
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

    /// Completes upload `id` of `bucket/key`: checks the listed parts against the stored ones, then writes the object
    /// they form under the request's condition, in one transaction that also drops the unlisted parts and ends the
    /// upload.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or an upload record this server did not write.
    pub async fn complete(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        id: UploadId,
        request: &Completion,
    ) -> Result<Completed> {
        let Some(declared) = self.find(bucket, key, id).await? else {
            return Ok(Completed::NoSuchUpload);
        };
        let algorithm = match declared
            .checksum_algorithm
            .as_deref()
            .map(ChecksumAlgorithm::parse)
        {
            None => None,
            Some(Some(Declared::Supported(algorithm))) => Some(algorithm),
            Some(_) => return Err(malformed("checksum_algorithm")),
        };
        let checksum = ObjectChecksum::declared(algorithm, declared.checksum_type.as_deref())
            .ok_or_else(|| malformed("checksum_type"))?;
        let rows = self.repository.all_parts(id).await?;
        let stored: BTreeMap<u16, PartRecord> = rows
            .iter()
            .map(|row| (row.number, row.record.clone()))
            .collect();
        let assembled = match assemble(&request.parts, &stored, checksum) {
            Ok(assembled) => assembled,
            Err(refusal) => return Ok(Completed::Refused(refusal)),
        };
        if request.size.is_some_and(|size| size != assembled.size) {
            return Ok(Completed::SizeMismatch);
        }
        if let Some((sent, value)) = &request.checksum
            && (*sent != checksum.algorithm() || *value != assembled.checksum)
        {
            return Ok(Completed::ChecksumMismatch);
        }
        // `assemble` found every listed number among the rows.
        let parts = request
            .parts
            .iter()
            .filter_map(|listed| rows.iter().find(|row| row.number == listed.number))
            .map(|row| Part {
                number: row.number,
                data: row.data,
                size: row.record.size,
            })
            .collect();
        let object = NewObject {
            size: assembled.size,
            etag: assembled.etag.clone(),
            headers: declared.headers,
            metadata: declared.metadata,
            checksums: [(
                checksum.algorithm().name().to_owned(),
                assembled.checksum.clone(),
            )]
            .into(),
            content: Content::Parts(Multipart {
                upload: id.bytes(),
                parts,
            }),
        };
        match self
            .objects
            .put(bucket, key, &object, &request.condition)
            .await?
        {
            Written::Committed(_) => Ok(Completed::Done {
                etag: assembled.etag,
                checksum: (checksum.algorithm(), assembled.checksum),
                composite: matches!(checksum, ObjectChecksum::Composite(_)),
            }),
            // An abort or another completion that won leaves no upload, whatever else the refusal looked like.
            _ if self.repository.upload(id).await?.is_none() => Ok(Completed::NoSuchUpload),
            other => Ok(Completed::NotWritten(other)),
        }
    }

    /// Ends upload `id`: its record and every part go, and every part's data is queued for reclamation.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn abort(&self, id: UploadId) -> Result<()> {
        self.repository.abort(id).await
    }
}
