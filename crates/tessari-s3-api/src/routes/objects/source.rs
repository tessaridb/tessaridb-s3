//! The source of a copy (CopyObject, UploadPartCopy): the object `x-amz-copy-source` names, read only when the
//! copy-source conditions hold, its verified bytes over any range, and those bytes written to a data file of the
//! copy's own — never the source's file, so either side can be deleted without touching the other.

use axum::body::Bytes;
use axum::http::HeaderMap;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tessari_s3_core::objects::conditions::{ReadConditions, ReadVerdict, evaluate};
use tessari_s3_core::objects::copy::{CopySource, CopySourceError};
use tessari_s3_storage::objects::{Content, ObjectService, StoredObject, Uploaded};
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use super::{body, data};
use crate::{Error, Result};

pub(super) fn text<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// Refuses the copy source's customer-key encryption headers, which `read_put_headers` does not see.
pub(crate) fn refuse_encrypted_source(headers: &HeaderMap) -> Result<()> {
    if headers.keys().any(|name| {
        name.as_str()
            .starts_with("x-amz-copy-source-server-side-encryption")
    }) {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            "server-side encryption is not implemented",
        ));
    }
    Ok(())
}

/// The object `x-amz-copy-source` names.
pub(crate) fn source_of(headers: &HeaderMap) -> Result<(BucketName, ObjectKey)> {
    let malformed = || {
        Error::new(
            ErrorCode::InvalidArgument,
            "x-amz-copy-source is not bucket/key",
        )
    };
    let source =
        CopySource::parse(text(headers, "x-amz-copy-source").unwrap_or("")).map_err(|error| {
            match error {
                CopySourceError::Version => Error::new(
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

/// The source object, when it exists and the copy-source conditions hold (412 when any does not, as S3 answers).
pub(crate) async fn read_source(
    objects: &ObjectService,
    headers: &HeaderMap,
    bucket: &BucketName,
    key: &ObjectKey,
) -> Result<StoredObject> {
    let object = match objects.get(bucket, key).await? {
        Err(()) => {
            return Err(Error::new(
                ErrorCode::NoSuchBucket,
                "the source bucket does not exist",
            ));
        }
        Ok(None) => {
            return Err(Error::new(
                ErrorCode::NoSuchKey,
                "the source key does not exist",
            ));
        }
        Ok(Some(object)) => object,
    };
    let conditions = ReadConditions {
        if_match: text(headers, "x-amz-copy-source-if-match"),
        if_none_match: text(headers, "x-amz-copy-source-if-none-match"),
        if_modified_since: text(headers, "x-amz-copy-source-if-modified-since"),
        if_unmodified_since: text(headers, "x-amz-copy-source-if-unmodified-since"),
    };
    if evaluate(&conditions, &object.etag, object.modified) != ReadVerdict::Proceed {
        return Err(Error::new(
            ErrorCode::PreconditionFailed,
            "at least one of the preconditions you specified did not hold",
        ));
    }
    Ok(object)
}

/// The source's verified bytes `start..=end`, in order; nothing for an empty object.
pub(crate) async fn bytes(
    objects: &ObjectService,
    object: &StoredObject,
    start: u64,
    end: u64,
) -> Result<BoxStream<'static, std::io::Result<Bytes>>> {
    Ok(match &object.content {
        _ if object.size == 0 => futures_util::stream::empty().boxed(),
        Content::Inline(inline) => {
            let range = usize::try_from(start)
                .ok()
                .zip(usize::try_from(end).ok())
                .and_then(|(from, to)| inline.get(from..=to))
                .ok_or_else(|| Error::new(ErrorCode::InternalError, "a range past inline bytes"))?;
            let slice = Bytes::copy_from_slice(range);
            futures_util::stream::once(async move { Ok(slice) }).boxed()
        }
        Content::Data(id) => body::blocks(objects, *id, object.size, start, end).await?,
        Content::Parts(multipart) => {
            let parts: Vec<([u8; 16], u64)> = multipart
                .parts
                .iter()
                .map(|part| (part.data, part.size))
                .collect();
            body::stream_parts(objects, &parts, start, end).await?
        }
    })
}

pub(crate) fn read_failed(error: &std::io::Error) -> Error {
    tracing::error!(error = %error, "a copy source could not be read whole");
    Error::new(
        ErrorCode::InternalError,
        "we encountered an internal error; please try again",
    )
}

/// Writes `stream` into a new data file of exactly `length` bytes; any failure releases the file.
pub(crate) async fn into_file(
    objects: &ObjectService,
    mut stream: BoxStream<'static, std::io::Result<Bytes>>,
    length: u64,
) -> Result<Uploaded> {
    if !objects.stores_data() {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            "objects larger than 128 KiB need a data directory (TESSARIDB_S3_DATA_DIR)",
        ));
    }
    let mut upload = objects.upload(length).await?;
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
        Ok(uploaded) if uploaded.size == length => Ok(uploaded),
        outcome => {
            data::release(objects, id).await;
            Err(outcome.err().unwrap_or_else(|| {
                Error::new(
                    ErrorCode::InternalError,
                    "the copy is not the source's length",
                )
            }))
        }
    }
}
