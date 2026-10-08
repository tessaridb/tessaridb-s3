//! CopyObject: a new object made of an existing object's bytes, read through the same verified paths as GetObject and
//! written through the same commit as PutObject. A copy never shares the source's data file or upload: it is a
//! single-part object of its own, so deleting either leaves the other whole. Its ETag is the MD5 of the bytes, and its
//! checksum is recomputed over them and — when the source holds a whole-object value of the same algorithm — checked
//! against the source's before anything commits.

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use futures_util::StreamExt;
use tessari_s3_constants::{INLINE_OBJECT_MAX, SINGLE_PUT_MAX};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared, Digests, Hashes};
use tessari_s3_core::objects::copy::Directive;
use tessari_s3_storage::objects::{Content, NewObject, ObjectService, StoredObject, Written};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::headers::read_put_headers;
use super::put::not_written;
use super::{data, source};
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::routes::quota;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

fn invalid(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

/// The copy's bytes: inline up to the inline size, as PutObject would store them; otherwise a new data file.
async fn copy_bytes(objects: &ObjectService, object: &StoredObject) -> Result<(Content, Digests)> {
    let last = object.size.saturating_sub(1);
    let mut stream = source::bytes(objects, object, 0, last).await?;
    let inline = usize::try_from(object.size)
        .ok()
        .filter(|size| *size <= INLINE_OBJECT_MAX);
    let Some(size) = inline else {
        let uploaded = source::into_file(objects, stream, object.size).await?;
        return Ok((Content::Data(uploaded.id), uploaded.digests));
    };
    let mut bytes = Vec::with_capacity(size);
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.map_err(|error| source::read_failed(&error))?);
    }
    let mut hashes = Hashes::new();
    hashes.update(&bytes);
    Ok((Content::Inline(bytes), hashes.finish()))
}

/// The algorithm the copy carries: the request's `x-amz-checksum-algorithm`, else the source's, else CRC64NVME.
fn algorithm_for(headers: &HeaderMap, object: &StoredObject) -> Result<ChecksumAlgorithm> {
    if let Some(name) = source::text(headers, "x-amz-checksum-algorithm") {
        return match ChecksumAlgorithm::parse(name) {
            Some(Declared::Supported(algorithm)) => Ok(algorithm),
            _ => Err(invalid(
                ErrorCode::NotImplemented,
                "that checksum algorithm is not implemented",
            )),
        };
    }
    Ok(object
        .checksums
        .keys()
        .find_map(|name| match ChecksumAlgorithm::parse(name) {
            Some(Declared::Supported(algorithm)) => Some(algorithm),
            _ => None,
        })
        .unwrap_or(ChecksumAlgorithm::Crc64Nvme))
}

/// `PUT /{bucket}/{key}` with `x-amz-copy-source`.
pub(crate) async fn copy(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    source::refuse_encrypted_source(call.headers)?;
    let headers = read_put_headers(call.headers)?;
    if headers.integrity.content_md5.is_some() || headers.integrity.checksum.is_some() {
        return Err(invalid(
            ErrorCode::InvalidRequest,
            "a copy has no body for Content-MD5 or a checksum to describe",
        ));
    }
    let (source_bucket, source_key) = source::source_of(call.headers)?;
    let directive = |name: &str| {
        Directive::parse(source::text(call.headers, name)).ok_or_else(|| {
            invalid(
                ErrorCode::InvalidArgument,
                "a directive must be COPY or REPLACE",
            )
        })
    };
    let metadata = directive("x-amz-metadata-directive")?;
    directive("x-amz-tagging-directive")?;
    let bucket = call.bucket()?;
    if source_bucket == *bucket && source_key == *key && metadata == Directive::Copy {
        return Err(invalid(
            ErrorCode::InvalidRequest,
            "This copy request is illegal because it is trying to copy an object to itself without changing the \
             object's metadata, storage class, website redirect location or encryption attributes.",
        ));
    }
    let objects = call.state.storage().objects();
    let object = source::read_source(objects, call.headers, &source_bucket, &source_key).await?;
    if object.size > SINGLE_PUT_MAX {
        return Err(invalid(
            ErrorCode::InvalidRequest,
            "The specified copy source is larger than the maximum allowable size for a copy source: 5368709120",
        ));
    }
    quota::admit(call, bucket, Some(key), object.size).await?;
    let algorithm = algorithm_for(call.headers, &object)?;
    let (content, digests) = copy_bytes(objects, &object).await?;
    let value = digests.value(algorithm).to_owned();
    // A whole-object value of the same algorithm must survive the copy; a composite one (`…-N`) covers part
    // digests and cannot be compared.
    let damaged = object
        .checksums
        .get(algorithm.name())
        .is_some_and(|stored| !stored.contains('-') && *stored != value);
    let (system, user) = match metadata {
        Directive::Copy => (object.headers, object.metadata),
        Directive::Replace => (headers.system, headers.metadata),
    };
    let copied = NewObject {
        size: object.size,
        etag: format!("\"{}\"", digests.md5_hex),
        headers: system,
        metadata: user,
        checksums: [(algorithm.name().to_owned(), value.clone())].into(),
        content,
    };
    if damaged {
        if let Content::Data(id) = copied.content {
            data::release(objects, id).await;
        }
        tracing::error!(
            bucket = source_bucket.as_str(),
            key = source_key.as_str(),
            "a copy source's bytes do not match its stored checksum"
        );
        return Err(invalid(
            ErrorCode::InternalError,
            "we encountered an internal error; please try again",
        ));
    }
    match objects
        .put(bucket, key, &copied, &headers.condition)
        .await?
    {
        Written::Committed(modified) => Ok(xml_response(
            StatusCode::OK,
            format!(
                "<CopyObjectResult xmlns=\"{S3_NAMESPACE}\"><ETag>{}</ETag><LastModified>{}</LastModified>\
                 <Checksum{name}>{}</Checksum{name}><ChecksumType>FULL_OBJECT</ChecksumType></CopyObjectResult>",
                escape(&copied.etag),
                modified.iso8601_millis(),
                escape(&value),
                name = algorithm.name()
            ),
        )),
        refused => Err(not_written(&refused)),
    }
}
