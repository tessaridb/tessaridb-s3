//! The source of a copy (CopyObject, UploadPartCopy): the object `x-amz-copy-source` names — its current version or
//! the one `?versionId=` names — read only when the copy-source conditions hold, its verified bytes over any range, and those bytes written to a data file of the
//! copy's own — never the source's file, so either side can be deleted without touching the other.

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue};
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use tessari_s3_core::objects::conditions::{ReadConditions, ReadVerdict, evaluate};
use tessari_s3_core::objects::copy::{CopySource, CopySourceError};
use tessari_s3_storage::objects::{Content, ObjectService, StoredObject, Uploaded, VersionRead};
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use tessari_s3_storage::encryption::CustomerKey;

use super::data::Seal;
use super::{body, data};
use crate::{Error, Result};

pub(super) fn text<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

/// What a copy reads: a bucket's key, at the version named or the current one.
pub(crate) struct CopyFrom {
    pub(crate) bucket: BucketName,
    pub(crate) key: ObjectKey,
    pub(crate) version: Option<String>,
}

/// The object `x-amz-copy-source` names.
pub(crate) fn source_of(headers: &HeaderMap) -> Result<CopyFrom> {
    let malformed = || {
        Error::new(
            ErrorCode::InvalidArgument,
            "x-amz-copy-source is not bucket/key",
        )
    };
    let source = CopySource::parse(text(headers, "x-amz-copy-source").unwrap_or(""))
        .map_err(|CopySourceError::Malformed| malformed())?;
    Ok(CopyFrom {
        bucket: BucketName::new(&source.bucket).map_err(|_| malformed())?,
        key: ObjectKey::new(&source.key).map_err(|_| malformed())?,
        version: source.version,
    })
}

/// Names the version a copy read as `x-amz-copy-source-version-id`; a source never versioned has none to name.
pub(crate) fn answer_version_id(headers: &mut HeaderMap, version: Option<&str>) {
    if let Some(value) = version.and_then(|version| HeaderValue::from_str(version).ok()) {
        headers.insert("x-amz-copy-source-version-id", value);
    }
}

/// The version `version` names: a delete marker is no source (400), an id naming nothing `NoSuchVersion`.
async fn named_version(
    objects: &ObjectService,
    from: &CopyFrom,
    version: &str,
) -> Result<StoredObject> {
    match objects
        .get_version(&from.bucket, &from.key, version)
        .await?
    {
        VersionRead::Object(object) => Ok(*object),
        VersionRead::Marker { .. } => Err(Error::new(
            ErrorCode::InvalidRequest,
            "The source of a copy request may not specifically refer to a delete marker by version id.",
        )),
        VersionRead::Absent => Err(Error::new(
            ErrorCode::NoSuchVersion,
            "The specified version does not exist.",
        )),
        VersionRead::Malformed => Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
        VersionRead::NoSuchBucket => Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the source bucket does not exist",
        )),
    }
}

/// The source key's current version; a current delete marker is as if the key were gone.
async fn current(objects: &ObjectService, from: &CopyFrom) -> Result<StoredObject> {
    match objects.get(&from.bucket, &from.key).await? {
        Err(()) => Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the source bucket does not exist",
        )),
        Ok(None) => Err(Error::new(
            ErrorCode::NoSuchKey,
            "the source key does not exist",
        )),
        Ok(Some(object)) => Ok(object),
    }
}

/// The source object, when it exists and the copy-source conditions hold (412 when any does not, as S3 answers).
pub(crate) async fn read_source(
    objects: &ObjectService,
    headers: &HeaderMap,
    from: &CopyFrom,
) -> Result<StoredObject> {
    let object = match &from.version {
        Some(version) => named_version(objects, from, version).await?,
        None => current(objects, from).await?,
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

/// The source's verified bytes `start..=end`, in order — opened with `customer`'s key when a customer's key sealed
/// it; nothing for an empty object.
pub(crate) async fn bytes(
    objects: &ObjectService,
    object: &StoredObject,
    customer: Option<&CustomerKey>,
    start: u64,
    end: u64,
) -> Result<BoxStream<'static, std::io::Result<Bytes>>> {
    Ok(match &object.content {
        _ if object.size == 0 => futures_util::stream::empty().boxed(),
        Content::Data(id) => {
            body::blocks(
                objects,
                *id,
                object.size,
                object.encryption.as_ref(),
                customer,
                start,
                end,
            )
            .await?
        }
        Content::Parts(multipart) => {
            let parts = &multipart.parts;
            body::stream_parts(objects, parts, customer, start, end).await?
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

/// Writes `stream` into a new data file of exactly `length` bytes, sealed as `seal` says; any failure releases the
/// file.
pub(crate) async fn into_file(
    objects: &ObjectService,
    mut stream: BoxStream<'static, std::io::Result<Bytes>>,
    length: u64,
    seal: Seal<'_>,
) -> Result<Uploaded> {
    let mut upload = match seal {
        Seal::Plain => objects.upload(length).await?,
        Seal::Under(sealing) => objects.upload_sealed(length, sealing).await?,
        Seal::Customer(key, customer) => objects.upload_customer(length, key, customer).await?,
    };
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
