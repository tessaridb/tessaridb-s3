//! PutObject. The declared length decides where the bytes go: up to the inline size the body is read whole (bounded)
//! into the metadata record; above it the body streams into a data file. Either way it is decoded if aws-chunked,
//! hashed in one pass, checked against every value the request declared, and committed with its condition as one
//! statement.

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use tessari_s3_constants::{CHUNKED_FRAMING_ALLOWANCE, INLINE_OBJECT_MAX, SINGLE_PUT_MAX};
use tessari_s3_core::auth::{ChunkedDecoder, PayloadHash};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Digests, Hashes};
use tessari_s3_storage::objects::{Content, NewObject, Written};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::data;
use super::headers::{Integrity, PutHeaders, read_put_headers};
use crate::pipeline::body::read_verified;
use crate::pipeline::call::Call;
use crate::pipeline::response::empty_response;
use crate::{Error, Result};

fn header_number(headers: &HeaderMap, name: &str) -> Result<Option<u64>> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidArgument,
                        format!("{name} is not a number"),
                    )
                })
        })
        .transpose()
}

const fn is_chunked(payload: &PayloadHash) -> bool {
    matches!(
        payload,
        PayloadHash::StreamingSigned
            | PayloadHash::StreamingSignedTrailer
            | PayloadHash::StreamingUnsignedTrailer
    )
}

/// The object's length as the request declares it: the decoded length of an aws-chunked body, the Content-Length
/// of any other.
pub(crate) fn declared_length(call: &Call<'_>) -> Result<u64> {
    if is_chunked(&call.verified.payload()) {
        header_number(call.headers, "x-amz-decoded-content-length")?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingContentLength,
                "x-amz-decoded-content-length is required",
            )
        })
    } else {
        header_number(call.headers, "content-length")?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingContentLength,
                "Content-Length is required",
            )
        })
    }
}

/// The decoded body of an object no larger than the inline size, with its trailing checksum when one was sent.
async fn read_inline(
    call: &Call<'_>,
    body: Body,
    length: usize,
) -> Result<(Vec<u8>, Option<String>)> {
    if is_chunked(&call.verified.payload()) {
        let raw = to_bytes(body, length.saturating_add(CHUNKED_FRAMING_ALLOWANCE))
            .await
            .map_err(|_| {
                Error::new(
                    ErrorCode::IncompleteBody,
                    "the aws-chunked body is longer than declared",
                )
            })?;
        let trailer_header = call
            .headers
            .get("x-amz-trailer")
            .and_then(|v| v.to_str().ok());
        let declared = u64::try_from(length).unwrap_or(u64::MAX);
        let mut decoder = ChunkedDecoder::new(call.verified.clone(), declared, trailer_header)?;
        let mut data = Vec::with_capacity(length);
        decoder.push(&raw, &mut data)?;
        let trailer = decoder.finish()?.map(|trailer| trailer.value);
        Ok((data, trailer))
    } else {
        let data = read_verified(body, call.verified, INLINE_OBJECT_MAX).await?;
        if data.len() != length {
            return Err(Error::new(
                ErrorCode::IncompleteBody,
                "the body is shorter than its Content-Length",
            ));
        }
        Ok((data.to_vec(), None))
    }
}

/// Checks the Content-MD5 and the declared checksum against the digests; answers the algorithm to store.
pub(crate) fn check(
    integrity: &Integrity,
    digests: &Digests,
    trailer: Option<String>,
) -> Result<ChecksumAlgorithm> {
    let bad_digest = |what: &str| {
        Error::new(
            ErrorCode::BadDigest,
            format!("the {what} you specified did not match the body"),
        )
    };
    if integrity.content_md5.is_some_and(|md5| md5 != digests.md5) {
        return Err(bad_digest("Content-MD5"));
    }
    match &integrity.checksum {
        Some(declared) => {
            let value = declared.header_value.clone().or(trailer).ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidRequest,
                    "a declared checksum arrived neither in a header nor a trailer",
                )
            })?;
            if value == digests.value(declared.algorithm) {
                Ok(declared.algorithm)
            } else {
                Err(bad_digest(&format!(
                    "{} checksum",
                    declared.algorithm.name()
                )))
            }
        }
        None => Ok(ChecksumAlgorithm::Crc64Nvme),
    }
}

/// `PUT /{bucket}/{key}`.
pub(crate) async fn put(call: &Call<'_>, key: &ObjectKey, body: Body) -> Result<Response<Body>> {
    let headers = read_put_headers(call.headers)?;
    let length = declared_length(call)?;
    if length > SINGLE_PUT_MAX {
        return Err(Error::new(
            ErrorCode::EntityTooLarge,
            "a single PUT carries at most 5 GiB; use a multipart upload",
        ));
    }
    match usize::try_from(length) {
        Ok(inline) if inline <= INLINE_OBJECT_MAX => {
            let (data, trailer) = read_inline(call, body, inline).await?;
            let mut hashes = Hashes::new();
            hashes.update(&data);
            let digests = hashes.finish();
            let algorithm = check(&headers.integrity, &digests, trailer)?;
            commit(
                call,
                key,
                headers,
                &digests,
                length,
                algorithm,
                Content::Inline(data),
            )
            .await
        }
        _ => {
            let objects = call.state.storage().objects();
            if !objects.stores_data() {
                return Err(Error::new(
                    ErrorCode::NotImplemented,
                    "objects larger than 128 KiB need a data directory (TESSARIDB_S3_DATA_DIR)",
                ));
            }
            let (uploaded, trailer) = data::receive(call, body, length).await?;
            let algorithm = match check(&headers.integrity, &uploaded.digests, trailer) {
                Ok(algorithm) => algorithm,
                Err(error) => {
                    data::release(objects, uploaded.id).await;
                    return Err(error);
                }
            };
            let content = Content::Data(uploaded.id);
            commit(
                call,
                key,
                headers,
                &uploaded.digests,
                uploaded.size,
                algorithm,
                content,
            )
            .await
        }
    }
}

/// Commits the object under its condition and answers as S3 does. A data object the store refused has already
/// released its file in the storage service.
async fn commit(
    call: &Call<'_>,
    key: &ObjectKey,
    headers: PutHeaders,
    digests: &Digests,
    size: u64,
    algorithm: ChecksumAlgorithm,
    content: Content,
) -> Result<Response<Body>> {
    let etag = format!("\"{}\"", digests.md5_hex);
    let declared_checksum = headers.integrity.checksum.is_some();
    let object = NewObject {
        size,
        etag: etag.clone(),
        headers: headers.system,
        metadata: headers.metadata,
        checksums: [(
            algorithm.name().to_owned(),
            digests.value(algorithm).to_owned(),
        )]
        .into(),
        content,
    };
    let bucket = call.bucket()?;
    match call
        .state
        .storage()
        .objects()
        .put(bucket, key, &object, &headers.condition)
        .await?
    {
        Written::Committed(_) => {
            let mut response = empty_response(StatusCode::OK);
            let response_headers = response.headers_mut();
            if let Ok(value) = HeaderValue::from_str(&etag) {
                response_headers.insert("etag", value);
            }
            if let Some(value) = declared_checksum
                .then(|| HeaderValue::from_str(digests.value(algorithm)).ok())
                .flatten()
            {
                response_headers.insert(algorithm.header(), value);
            }
            Ok(response)
        }
        refused => Err(not_written(&refused)),
    }
}

/// The S3 error for an object write that did not commit.
pub(super) fn not_written(written: &Written) -> Error {
    let (code, message) = match written {
        Written::NoSuchBucket => (
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        ),
        Written::NoSuchKey => (ErrorCode::NoSuchKey, "the specified key does not exist"),
        Written::DataReclaimed => (
            ErrorCode::InternalError,
            "the upload took longer than its data is kept; please try again",
        ),
        Written::PreconditionFailed => (
            ErrorCode::PreconditionFailed,
            "at least one of the preconditions you specified did not hold",
        ),
        Written::Committed(_) => (
            ErrorCode::InternalError,
            "a committed write was reported as refused",
        ),
    };
    Error::new(code, message)
}
