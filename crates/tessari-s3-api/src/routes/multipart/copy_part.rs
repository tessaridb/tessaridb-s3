//! UploadPartCopy: a part made of a source object's bytes — a range of them or all — read through the same verified
//! paths and conditions as CopyObject, written to a data file of the part's own, and committed exactly as an uploaded
//! part is. The upload is checked before the source is read.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_constants::SINGLE_PUT_MAX;
use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_core::objects::copy::{CopyRangeError, copy_range};
use tessari_s3_storage::multipart::{NewPart, PartWritten};
use tessari_s3_types::{ErrorCode, ObjectKey, Timestamp};

use super::{no_such_upload, part_number, upload_id};
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::routes::objects::headers::read_integrity;
use crate::routes::objects::source;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

/// The bytes `first..=last` of a source of `size` bytes the request asks for, and how many there are.
fn wanted(call: &Call<'_>, size: u64) -> Result<(u64, u64, u64)> {
    let Some(range) = call
        .headers
        .get("x-amz-copy-source-range")
        .map(|value| value.to_str().unwrap_or(""))
    else {
        let last = size.saturating_sub(1);
        return Ok((0, last, size));
    };
    let (first, last) = copy_range(range, size).map_err(|error| match error {
        CopyRangeError::Malformed => Error::new(
            ErrorCode::InvalidArgument,
            "x-amz-copy-source-range must be bytes=first-last",
        ),
        CopyRangeError::OutOfRange => Error::new(
            ErrorCode::InvalidRequest,
            format!("Range specified is not valid for source object of size: {size}"),
        ),
    })?;
    let length = last
        .checked_sub(first)
        .and_then(|span| span.checked_add(1))
        .ok_or_else(|| Error::new(ErrorCode::InvalidArgument, "range"))?;
    Ok((first, last, length))
}

/// `PUT /{bucket}/{key}?partNumber=N&uploadId=X` with `x-amz-copy-source`.
pub(crate) async fn copy_part(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let id = upload_id(call)?;
    let number = part_number(call)?;
    source::refuse_encrypted_source(call.headers)?;
    let integrity = read_integrity(call.headers)?;
    if integrity.content_md5.is_some() || integrity.checksum.is_some() {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "a part copy has no body for Content-MD5 or a checksum to describe",
        ));
    }
    let (source_bucket, source_key) = source::source_of(call.headers)?;
    let bucket = call.bucket()?;
    let storage = call.state.storage();
    let Some(declared) = storage.multipart().find(bucket, key, id).await? else {
        return Err(no_such_upload());
    };
    let objects = storage.objects();
    let object = source::read_source(objects, call.headers, &source_bucket, &source_key).await?;
    let (first, last, length) = wanted(call, object.size)?;
    if length > SINGLE_PUT_MAX {
        return Err(Error::new(
            ErrorCode::EntityTooLarge,
            "a part carries at most 5 GiB",
        ));
    }
    let stream = source::bytes(objects, &object, first, last).await?;
    let uploaded = source::into_file(objects, stream, length).await?;
    let etag = format!("\"{}\"", uploaded.digests.md5_hex);
    let part = NewPart {
        data: uploaded.id,
        size: uploaded.size,
        etag: etag.clone(),
        // Every algorithm, as for an uploaded part: Complete builds the object's checksum from them.
        checksums: ChecksumAlgorithm::ALL
            .into_iter()
            .map(|each| {
                (
                    each.name().to_owned(),
                    uploaded.digests.value(each).to_owned(),
                )
            })
            .collect(),
    };
    match storage
        .multipart()
        .write_part(bucket, key, id, number, &part)
        .await?
    {
        PartWritten::Committed => {
            let since = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let now = Timestamp {
                seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
                nanos: since.subsec_nanos(),
            };
            let mut out = format!(
                "<CopyPartResult xmlns=\"{S3_NAMESPACE}\"><ETag>{}</ETag><LastModified>{}</LastModified>",
                escape(&etag),
                now.iso8601_millis()
            );
            if let Some(name) = declared.checksum_algorithm.as_deref()
                && let Some(value) = part.checksums.get(name)
            {
                out.push_str(&format!(
                    "<Checksum{0}>{1}</Checksum{0}>",
                    escape(name),
                    escape(value)
                ));
            }
            out.push_str("</CopyPartResult>");
            Ok(xml_response(StatusCode::OK, out))
        }
        PartWritten::NoSuchUpload => Err(no_such_upload()),
        PartWritten::DataReclaimed => Err(Error::new(
            ErrorCode::InternalError,
            "the part took longer than its data is kept; please try again",
        )),
    }
}
