//! Replicas a source bucket sends (S3 ref 11 M4): PutObject and DeleteObject carrying the version id the source gave
//! the version keep that id on this bucket. The header has no S3 equivalent; who may send it is decided like a request
//! naming a version, so it needs who administers the bucket.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_storage::objects::Replicated;
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::with_delete_marker;
use crate::pipeline::call::Call;
use crate::pipeline::response::empty_response;
use crate::{Error, Result};

/// The header carrying a replica's source version id.
pub(crate) const REPLICA_VERSION: &str = "x-tessaridb-replica-version-id";

/// The source version id a replica write carries, if the request is one.
pub(super) fn replica_version<'a>(call: &'a Call<'_>) -> Result<Option<&'a str>> {
    call.headers
        .get(REPLICA_VERSION)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| Error::new(ErrorCode::InvalidArgument, "Invalid version id specified"))
        })
        .transpose()
}

/// The S3 error for a replica that was not stored, or `None` when it is here (stored now or before).
pub(super) fn refused(outcome: Replicated) -> Option<Error> {
    match outcome {
        Replicated::Stored | Replicated::Kept => None,
        Replicated::Conflict => Some(Error::new(
            ErrorCode::OperationAborted,
            "another version holds this replica's place in the key's history",
        )),
        Replicated::NoSuchBucket => Some(Error::new(
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        )),
        Replicated::NotAVersionId => Some(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
    }
}

/// `DELETE /{bucket}/{key}` carrying a replica's source version id: the delete marker is stored under that id.
pub(super) async fn delete(
    call: &Call<'_>,
    key: &ObjectKey,
    version_id: &str,
) -> Result<Response<Body>> {
    let outcome = call
        .state
        .storage()
        .objects()
        .mark_replica(call.bucket()?, key, version_id)
        .await?;
    match refused(outcome) {
        Some(error) => Err(error),
        None => Ok(with_delete_marker(
            empty_response(StatusCode::NO_CONTENT),
            Some(version_id),
        )),
    }
}
