//! Multipart uploads: CreateMultipartUpload, UploadPart, AbortMultipartUpload and ListParts (CompleteMultipartUpload
//! and ListMultipartUploads in their own modules). An upload id that is
//! malformed, unknown, or issued for another key answers `NoSuchUpload`; a part's body streams into its own data file
//! exactly as a large PutObject's does.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode};
use tessari_s3_constants::{LIST_PARTS_MAX, SINGLE_PUT_MAX};
use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_storage::multipart::{Created, NewPart, NewUpload, PartWritten};
use tessari_s3_types::{ErrorCode, ObjectKey, PartNumber, UploadId};

use super::objects::data;
use super::objects::headers::{read_integrity, read_put_headers};
use super::objects::put::{check, declared_length};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, escape, xml_text};
use crate::{Error, Result};

mod complete;
mod declared;
mod uploads;

pub(crate) use complete::complete;
use declared::declared_checksum;
pub(crate) use uploads::list_uploads;

fn no_such_upload() -> Error {
    Error::new(
        ErrorCode::NoSuchUpload,
        "the specified multipart upload does not exist",
    )
}

/// The `uploadId` query parameter; anything this server could not have issued is `NoSuchUpload`.
fn upload_id(call: &Call<'_>) -> Result<UploadId> {
    call.query_value("uploadId")
        .and_then(|text| UploadId::parse(text).ok())
        .ok_or_else(no_such_upload)
}

/// A non-negative integer query parameter, at most `max`.
fn bounded(call: &Call<'_>, name: &str, default: usize, max: usize) -> Result<usize> {
    match call.query_value(name) {
        None => Ok(default),
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|value| *value <= max)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidArgument,
                    format!("{name} must be an integer from 0 to {max}"),
                )
            }),
    }
}

/// A key as XML text; a key XML 1.0 cannot carry is refused rather than written as invalid XML.
fn key_text(key: &ObjectKey) -> Result<String> {
    xml_text(key.as_str()).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            "the key cannot be carried in an XML response",
        )
    })
}

/// `POST /{bucket}/{key}?uploads`.
pub(crate) async fn create(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let headers = read_put_headers(call.headers)?;
    let (checksum_algorithm, checksum_type) = declared_checksum(call)?;
    let upload = NewUpload {
        headers: headers.system,
        metadata: headers.metadata,
        checksum_algorithm,
        checksum_type,
    };
    let bucket = call.bucket()?;
    let Created::Created(id) = call
        .state
        .storage()
        .multipart()
        .create(bucket, key, &upload)
        .await?
    else {
        return Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        ));
    };
    let mut response = xml_response(
        StatusCode::OK,
        format!(
            "<InitiateMultipartUploadResult xmlns=\"{S3_NAMESPACE}\"><Bucket>{}</Bucket><Key>{}</Key>\
             <UploadId>{}</UploadId></InitiateMultipartUploadResult>",
            escape(bucket.as_str()),
            key_text(key)?,
            id.hex()
        ),
    );
    for (name, value) in [
        ("x-amz-checksum-algorithm", &upload.checksum_algorithm),
        ("x-amz-checksum-type", &upload.checksum_type),
    ] {
        if let Some(value) = value.as_deref().and_then(|v| HeaderValue::from_str(v).ok()) {
            response.headers_mut().insert(name, value);
        }
    }
    Ok(response)
}

/// `PUT /{bucket}/{key}?partNumber=N&uploadId=X`.
pub(crate) async fn upload_part(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    let id = upload_id(call)?;
    let number = PartNumber::parse(call.query_value("partNumber").unwrap_or("")).map_err(|_| {
        Error::new(
            ErrorCode::InvalidArgument,
            "partNumber must be an integer from 1 to 10000",
        )
    })?;
    let integrity = read_integrity(call.headers)?;
    let length = declared_length(call)?;
    if length > SINGLE_PUT_MAX {
        return Err(Error::new(
            ErrorCode::EntityTooLarge,
            "a part carries at most 5 GiB",
        ));
    }
    let storage = call.state.storage();
    if !storage.objects().stores_data() {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            "multipart uploads need a data directory (TESSARIDB_S3_DATA_DIR)",
        ));
    }
    let bucket = call.bucket()?;
    // Refuse before reading the body: a part for no upload is never written to a drive.
    if !storage.multipart().exists(bucket, key, id).await? {
        return Err(no_such_upload());
    }
    let (uploaded, trailer) = data::receive(call, body, length).await?;
    let algorithm = match check(&integrity, &uploaded.digests, trailer) {
        Ok(algorithm) => algorithm,
        Err(error) => {
            data::release(storage.objects(), uploaded.id).await;
            return Err(error);
        }
    };
    let etag = format!("\"{}\"", uploaded.digests.md5_hex);
    let part = NewPart {
        data: uploaded.id,
        size: uploaded.size,
        etag: etag.clone(),
        // Every algorithm, whichever the client checked: Complete builds the object's checksum from them.
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
            let mut response = empty_response(StatusCode::OK);
            if let Ok(value) = HeaderValue::from_str(&etag) {
                response.headers_mut().insert("etag", value);
            }
            if integrity.checksum.is_some()
                && let Ok(value) = HeaderValue::from_str(uploaded.digests.value(algorithm))
            {
                response.headers_mut().insert(algorithm.header(), value);
            }
            Ok(response)
        }
        PartWritten::NoSuchUpload => Err(no_such_upload()),
        PartWritten::DataReclaimed => Err(Error::new(
            ErrorCode::InternalError,
            "the part took longer than its data is kept; please try again",
        )),
    }
}

/// `DELETE /{bucket}/{key}?uploadId=X`.
pub(crate) async fn abort(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let id = upload_id(call)?;
    let multipart = call.state.storage().multipart();
    if !multipart.exists(call.bucket()?, key, id).await? {
        return Err(no_such_upload());
    }
    multipart.abort(id).await?;
    Ok(empty_response(StatusCode::NO_CONTENT))
}

/// `GET /{bucket}/{key}?uploadId=X`, paged by `part-number-marker` and `max-parts`.
pub(crate) async fn list_parts(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let id = upload_id(call)?;
    let max = bounded(call, "max-parts", LIST_PARTS_MAX, LIST_PARTS_MAX)?;
    let marker = bounded(call, "part-number-marker", 0, 10_000)?;
    let marker = u16::try_from(marker).map_err(|_| no_such_upload())?;
    let bucket = call.bucket()?;
    let multipart = call.state.storage().multipart();
    let Some(declared) = multipart.find(bucket, key, id).await? else {
        return Err(no_such_upload());
    };
    let page = multipart.parts(id, marker, max).await?;
    let owner = escape(call.verified.access_key());
    let mut out = format!(
        "<ListPartsResult xmlns=\"{S3_NAMESPACE}\"><Bucket>{}</Bucket><Key>{}</Key><UploadId>{}</UploadId>\
         <PartNumberMarker>{marker}</PartNumberMarker>",
        escape(bucket.as_str()),
        key_text(key)?,
        id.hex()
    );
    if let Some(last) = page.parts.last() {
        let _ = write!(
            out,
            "<NextPartNumberMarker>{}</NextPartNumberMarker>",
            last.number.get()
        );
    }
    let _ = write!(
        out,
        "<MaxParts>{max}</MaxParts><IsTruncated>{}</IsTruncated>",
        page.truncated
    );
    for part in &page.parts {
        let _ = write!(
            out,
            "<Part><PartNumber>{}</PartNumber><LastModified>{}</LastModified><ETag>{}</ETag><Size>{}</Size>",
            part.number.get(),
            part.modified.iso8601_millis(),
            escape(&part.etag),
            part.size
        );
        // A part holds every algorithm's value; the listing shows the one the upload declared.
        let shown = part
            .checksums
            .iter()
            .filter(|(algorithm, _)| declared.checksum_algorithm.as_ref() == Some(*algorithm));
        for (algorithm, value) in shown {
            let _ = write!(
                out,
                "<Checksum{0}>{1}</Checksum{0}>",
                escape(algorithm),
                escape(value)
            );
        }
        out.push_str("</Part>");
    }
    let _ = write!(
        out,
        "<Initiator><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Initiator>\
         <Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Owner>\
         <StorageClass>STANDARD</StorageClass></ListPartsResult>"
    );
    Ok(xml_response(StatusCode::OK, out))
}
