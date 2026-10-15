//! Object operations: PutObject, CopyObject, GetObject, HeadObject, GetObjectTagging, DeleteObject and DeleteObjects.

pub(crate) mod body;
mod copy;
pub(crate) mod data;
mod delete_many;
pub(crate) mod headers;
pub(crate) mod put;
mod read;
mod replica;
pub(crate) mod source;

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode};
use tessari_s3_storage::objects::{Removed, VersionRemoved};
use tessari_s3_types::{ErrorCode, ObjectKey};

use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, with_version, xml_response};
use crate::xml::S3_NAMESPACE;
use crate::{Error, Result};

pub(crate) use copy::copy;
pub(crate) use delete_many::delete_many;
pub(crate) use put::put;
pub(crate) use read::read;
pub(crate) use replica::REPLICA_VERSION;

/// The key an object operation addresses. Dispatch guarantees one; its absence is a defect.
pub(crate) fn key<'a>(call: &'a Call<'_>) -> Result<&'a ObjectKey> {
    call.addressed.key.as_ref().ok_or_else(|| {
        Error::new(
            ErrorCode::InternalError,
            "an object operation addressed no key",
        )
    })
}

/// `DELETE /{bucket}/{key}` — 204 whether or not the key held an object, as in S3.
pub(crate) async fn delete(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    for header in [
        "if-match",
        "x-amz-if-match-last-modified-time",
        "x-amz-if-match-size",
    ] {
        if call.headers.contains_key(header) {
            return Err(Error::new(
                ErrorCode::NotImplemented,
                "conditional deletes are not implemented",
            ));
        }
    }
    if let Some(version_id) = replica::replica_version(call)? {
        return replica::delete(call, key, version_id).await;
    }
    if call.query.iter().any(|(name, _)| name == "versionId") {
        return delete_version(call, key).await;
    }
    match call
        .state
        .storage()
        .objects()
        .delete(call.bucket()?, key)
        .await?
    {
        // A delete that added a marker names it (S3 ref 08 M3).
        Removed::Done { marker } => Ok(match marker {
            Some(id) => with_delete_marker(empty_response(StatusCode::NO_CONTENT), Some(&id)),
            None => empty_response(StatusCode::NO_CONTENT),
        }),
        Removed::NoSuchBucket => Err(no_such_bucket()),
    }
}

/// `DELETE /{bucket}/{key}?versionId=…` — that version goes for good; a version that is not there is already gone.
async fn delete_version(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let version = call.query_value("versionId").unwrap_or_default();
    let removed = call
        .state
        .storage()
        .objects()
        .delete_version(call.bucket()?, key, version)
        .await?;
    let response = with_version(empty_response(StatusCode::NO_CONTENT), Some(version));
    match removed {
        VersionRemoved::Done { marker: true } => Ok(with_delete_marker(response, None)),
        VersionRemoved::Done { marker: false } | VersionRemoved::Absent => Ok(response),
        VersionRemoved::Malformed => Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
        VersionRemoved::NoSuchBucket => Err(no_such_bucket()),
    }
}

/// Adds `x-amz-delete-marker: true`, with the marker's id when it is not already on the response.
fn with_delete_marker(mut response: Response<Body>, id: Option<&str>) -> Response<Body> {
    response
        .headers_mut()
        .insert("x-amz-delete-marker", HeaderValue::from_static("true"));
    with_version(response, id)
}

fn no_such_bucket() -> Error {
    Error::new(
        ErrorCode::NoSuchBucket,
        "the specified bucket does not exist",
    )
}

/// `GET /{bucket}/{key}?tagging`. Nothing writes a tag here — PutObject refuses `x-amz-tagging` and PutObjectTagging is
/// not implemented — so an existing object's tag set is empty, and that is the answer.
pub(crate) async fn tagging(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    if call.query.iter().any(|(name, _)| name == "versionId") {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            "versionId is not implemented",
        ));
    }
    match call
        .state
        .storage()
        .objects()
        .get(call.bucket()?, key)
        .await?
    {
        Err(()) => Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        )),
        Ok(None) => Err(Error::new(
            ErrorCode::NoSuchKey,
            "the specified key does not exist",
        )),
        Ok(Some(_)) => Ok(xml_response(
            StatusCode::OK,
            format!("<Tagging xmlns=\"{S3_NAMESPACE}\"><TagSet></TagSet></Tagging>"),
        )),
    }
}
