//! CopyObject: a new object made of an existing object's bytes, read through the same verified paths as GetObject and
//! written through the same commit as PutObject. A copy never shares the source's data file or upload: it is a
//! single-part object of its own, so deleting either leaves the other whole. Its ETag is the MD5 of the bytes, and its
//! checksum is recomputed over them and — when the source holds a whole-object value of the same algorithm — checked
//! against the source's before anything commits.

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Response, StatusCode};
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tessari_s3_constants::{INLINE_OBJECT_MAX, SINGLE_PUT_MAX};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared, Digests, Hashes};
use tessari_s3_core::objects::conditions::{ReadConditions, ReadVerdict, evaluate};
use tessari_s3_core::objects::copy::{CopySource, CopySourceError, Directive};
use tessari_s3_storage::objects::{Content, NewObject, ObjectService, StoredObject, Written};
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use super::data;
use super::headers::read_put_headers;
use super::put::not_written;
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

fn invalid(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

fn text<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// The object `x-amz-copy-source` names.
fn source_of(headers: &HeaderMap) -> Result<(BucketName, ObjectKey)> {
    let malformed = || {
        invalid(
            ErrorCode::InvalidArgument,
            "x-amz-copy-source is not bucket/key",
        )
    };
    let source =
        CopySource::parse(text(headers, "x-amz-copy-source").unwrap_or("")).map_err(|error| {
            match error {
                CopySourceError::Version => invalid(
                    ErrorCode::NotImplemented,
                    "copying a version is not implemented",
                ),
                CopySourceError::Malformed => malformed(),
            }
        })?;
    let bucket = BucketName::new(&source.bucket).map_err(|_| malformed())?;
    let key = ObjectKey::new(&source.key).map_err(|_| malformed())?;
    Ok((bucket, key))
}

/// The copy-source conditions, which S3 answers 412 whenever they do not all hold.
fn source_holds(headers: &HeaderMap, object: &StoredObject) -> bool {
    let conditions = ReadConditions {
        if_match: text(headers, "x-amz-copy-source-if-match"),
        if_none_match: text(headers, "x-amz-copy-source-if-none-match"),
        if_modified_since: text(headers, "x-amz-copy-source-if-modified-since"),
        if_unmodified_since: text(headers, "x-amz-copy-source-if-unmodified-since"),
    };
    evaluate(&conditions, &object.etag, object.modified) == ReadVerdict::Proceed
}

/// The source's verified bytes, in order.
async fn source_bytes(
    objects: &ObjectService,
    object: &StoredObject,
) -> Result<BoxStream<'static, std::io::Result<Bytes>>> {
    let last = object.size.saturating_sub(1);
    Ok(match &object.content {
        _ if object.size == 0 => futures_util::stream::empty().boxed(),
        Content::Inline(bytes) => {
            let bytes = Bytes::copy_from_slice(bytes);
            futures_util::stream::once(async move { Ok(bytes) }).boxed()
        }
        Content::Data(id) => data::blocks(objects, *id, object.size, 0, last).await?,
        Content::Parts(multipart) => {
            let parts: Vec<([u8; 16], u64)> = multipart
                .parts
                .iter()
                .map(|part| (part.data, part.size))
                .collect();
            data::stream_parts(objects, &parts, 0, last).await?
        }
    })
}

fn read_failed(error: &std::io::Error) -> Error {
    tracing::error!(error = %error, "a copy source could not be read whole");
    invalid(
        ErrorCode::InternalError,
        "we encountered an internal error; please try again",
    )
}

/// The copy's bytes: inline up to the inline size, as PutObject would store them; otherwise a new data file.
async fn copy_bytes(objects: &ObjectService, object: &StoredObject) -> Result<(Content, Digests)> {
    let mut stream = source_bytes(objects, object).await?;
    let inline = usize::try_from(object.size)
        .ok()
        .filter(|size| *size <= INLINE_OBJECT_MAX);
    if let Some(size) = inline {
        let mut bytes = Vec::with_capacity(size);
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.map_err(|error| read_failed(&error))?);
        }
        let mut hashes = Hashes::new();
        hashes.update(&bytes);
        return Ok((Content::Inline(bytes), hashes.finish()));
    }
    if !objects.stores_data() {
        return Err(invalid(
            ErrorCode::NotImplemented,
            "objects larger than 128 KiB need a data directory (TESSARIDB_S3_DATA_DIR)",
        ));
    }
    let mut upload = objects.upload().await?;
    let id = upload.id();
    let mut written = Ok(());
    while let Some(chunk) = stream.next().await {
        written = match chunk {
            Ok(bytes) => upload.append(&bytes).await.map_err(Error::from),
            Err(error) => Err(read_failed(&error)),
        };
        if written.is_err() {
            break;
        }
    }
    let finished = match written {
        Ok(()) => upload.finish().await.map_err(Error::from),
        Err(error) => {
            drop(upload);
            Err(error)
        }
    };
    match finished {
        Ok(uploaded) if uploaded.size == object.size => Ok((Content::Data(id), uploaded.digests)),
        outcome => {
            data::release(objects, id).await;
            Err(outcome.err().unwrap_or_else(|| {
                invalid(
                    ErrorCode::InternalError,
                    "the copy is not the source's length",
                )
            }))
        }
    }
}

/// The algorithm the copy carries: the request's `x-amz-checksum-algorithm`, else the source's, else CRC64NVME.
fn algorithm_for(headers: &HeaderMap, object: &StoredObject) -> Result<ChecksumAlgorithm> {
    if let Some(name) = text(headers, "x-amz-checksum-algorithm") {
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
    // `read_put_headers` refuses `x-amz-server-side-encryption*`, not the copy source's own customer-key headers.
    if call.headers.keys().any(|name| {
        name.as_str()
            .starts_with("x-amz-copy-source-server-side-encryption")
    }) {
        return Err(invalid(
            ErrorCode::NotImplemented,
            "server-side encryption is not implemented",
        ));
    }
    let headers = read_put_headers(call.headers)?;
    if headers.integrity.content_md5.is_some() || headers.integrity.checksum.is_some() {
        return Err(invalid(
            ErrorCode::InvalidRequest,
            "a copy has no body for Content-MD5 or a checksum to describe",
        ));
    }
    let (source_bucket, source_key) = source_of(call.headers)?;
    let directive = |name: &str| {
        Directive::parse(text(call.headers, name)).ok_or_else(|| {
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
    let object = match objects.get(&source_bucket, &source_key).await? {
        Err(()) => {
            return Err(invalid(
                ErrorCode::NoSuchBucket,
                "the source bucket does not exist",
            ));
        }
        Ok(None) => {
            return Err(invalid(
                ErrorCode::NoSuchKey,
                "the source key does not exist",
            ));
        }
        Ok(Some(object)) => object,
    };
    if !source_holds(call.headers, &object) {
        return Err(invalid(
            ErrorCode::PreconditionFailed,
            "at least one of the preconditions you specified did not hold",
        ));
    }
    if object.size > SINGLE_PUT_MAX {
        return Err(invalid(
            ErrorCode::InvalidRequest,
            "The specified copy source is larger than the maximum allowable size for a copy source: 5368709120",
        ));
    }
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
