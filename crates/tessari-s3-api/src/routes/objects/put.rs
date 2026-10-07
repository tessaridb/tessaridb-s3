//! PutObject for objects up to the inline size: the body is read whole (bounded), decoded if aws-chunked, hashed
//! once, checked against every value the request declared, and committed with its condition as one statement.

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use tessari_s3_constants::{CHUNKED_FRAMING_ALLOWANCE, INLINE_OBJECT_MAX};
use tessari_s3_core::auth::{ChunkedDecoder, PayloadHash};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Hashes};
use tessari_s3_storage::objects::{NewObject, Written};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::headers::read_put_headers;
use crate::pipeline::body::read_verified;
use crate::pipeline::call::Call;
use crate::pipeline::response::empty_response;
use crate::{Error, Result};

fn too_large() -> Error {
    Error::new(
        ErrorCode::NotImplemented,
        "objects larger than 128 KiB are not stored by this build yet",
    )
}

fn header_number(headers: &HeaderMap, name: &str) -> Result<Option<usize>> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|text| text.trim().parse::<usize>().ok())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidArgument,
                        format!("{name} is not a number"),
                    )
                })
        })
        .transpose()
}

/// The decoded body, at most the inline size, with its trailing checksum when one was sent.
async fn read_body(call: &Call<'_>, body: Body) -> Result<(Vec<u8>, Option<String>)> {
    match call.verified.payload() {
        PayloadHash::StreamingSigned
        | PayloadHash::StreamingSignedTrailer
        | PayloadHash::StreamingUnsignedTrailer => {
            let decoded_len = header_number(call.headers, "x-amz-decoded-content-length")?
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::MissingContentLength,
                        "x-amz-decoded-content-length is required",
                    )
                })?;
            if decoded_len > INLINE_OBJECT_MAX {
                return Err(too_large());
            }
            let raw = to_bytes(body, decoded_len.saturating_add(CHUNKED_FRAMING_ALLOWANCE))
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
            let declared = u64::try_from(decoded_len).map_err(|_| too_large())?;
            let mut decoder = ChunkedDecoder::new(call.verified.clone(), declared, trailer_header)?;
            let mut data = Vec::with_capacity(decoded_len);
            decoder.push(&raw, &mut data)?;
            let trailer = decoder.finish()?.map(|trailer| trailer.value);
            Ok((data, trailer))
        }
        PayloadHash::Sha256(_) | PayloadHash::Unsigned => {
            let length = header_number(call.headers, "content-length")?.ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingContentLength,
                    "Content-Length is required",
                )
            })?;
            if length > INLINE_OBJECT_MAX {
                return Err(too_large());
            }
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
}

/// `PUT /{bucket}/{key}`.
pub(crate) async fn put(call: &Call<'_>, key: &ObjectKey, body: Body) -> Result<Response<Body>> {
    let headers = read_put_headers(call.headers)?;
    let (data, trailer_value) = read_body(call, body).await?;
    let mut hashes = Hashes::new();
    hashes.update(&data);
    let digests = hashes.finish();
    let bad_digest = |what: &str| {
        Error::new(
            ErrorCode::BadDigest,
            format!("the {what} you specified did not match the body"),
        )
    };
    if headers.content_md5.is_some_and(|md5| md5 != digests.md5) {
        return Err(bad_digest("Content-MD5"));
    }
    let stored_algorithm = match &headers.checksum {
        Some(declared) => {
            let value = declared
                .header_value
                .clone()
                .or(trailer_value)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidRequest,
                        "a declared checksum arrived neither in a header nor a trailer",
                    )
                })?;
            if value != digests.value(declared.algorithm) {
                return Err(bad_digest(&format!(
                    "{} checksum",
                    declared.algorithm.name()
                )));
            }
            declared.algorithm
        }
        None => ChecksumAlgorithm::Crc64Nvme,
    };
    let etag = format!("\"{}\"", digests.md5_hex);
    let object = NewObject {
        size: u64::try_from(data.len()).map_err(|_| too_large())?,
        etag: etag.clone(),
        headers: headers.system,
        metadata: headers.metadata,
        checksums: [(
            stored_algorithm.name().to_owned(),
            digests.value(stored_algorithm).to_owned(),
        )]
        .into(),
        inline: data,
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
            if let Some(value) = headers
                .checksum
                .as_ref()
                .and_then(|_| HeaderValue::from_str(digests.value(stored_algorithm)).ok())
            {
                response_headers.insert(stored_algorithm.header(), value);
            }
            Ok(response)
        }
        Written::NoSuchBucket => Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        )),
        Written::NoSuchKey => Err(Error::new(
            ErrorCode::NoSuchKey,
            "the specified key does not exist",
        )),
        Written::PreconditionFailed => Err(Error::new(
            ErrorCode::PreconditionFailed,
            "at least one of the preconditions you specified did not hold",
        )),
    }
}
