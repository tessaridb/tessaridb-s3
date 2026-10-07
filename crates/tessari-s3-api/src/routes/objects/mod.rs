//! Object operations: PutObject, GetObject, HeadObject and DeleteObject.

mod data;
mod headers;
mod put;
mod read;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_storage::objects::Removed;
use tessari_s3_types::{ErrorCode, ObjectKey};

use crate::pipeline::call::Call;
use crate::pipeline::response::empty_response;
use crate::{Error, Result};

pub(crate) use put::put;
pub(crate) use read::read;

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
        .delete(call.bucket()?, key)
        .await?
    {
        Removed::Done => Ok(empty_response(StatusCode::NO_CONTENT)),
        Removed::NoSuchBucket => Err(Error::new(
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        )),
    }
}
