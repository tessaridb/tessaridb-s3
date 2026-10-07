//! The operations with handlers, one module per domain. An operation marked implemented in the core's support
//! table but missing here is refused as an internal error rather than routed anywhere else.

use axum::body::Body;
use axum::http::Response;
use tessari_s3_core::dispatch::Operation;
use tessari_s3_types::ErrorCode;

use crate::pipeline::call::Call;
use crate::{Error, Result};

pub mod buckets;
pub mod multipart;
pub mod objects;

/// Runs the handler for `operation`.
///
/// # Errors
/// The handler's S3 error, or `InternalError` for an implemented operation with no handler.
pub(crate) async fn route(
    operation: Operation,
    call: &Call<'_>,
    body: Body,
) -> Result<Response<Body>> {
    match operation {
        Operation::CreateBucket => buckets::create(call, body).await,
        Operation::HeadBucket => buckets::head(call).await,
        Operation::DeleteBucket => buckets::delete(call).await,
        Operation::ListBuckets => buckets::list(call).await,
        Operation::GetBucketLocation => buckets::location(call).await,
        Operation::PutObject => objects::put(call, objects::key(call)?, body).await,
        Operation::GetObject => objects::read(call, objects::key(call)?, false).await,
        Operation::HeadObject => objects::read(call, objects::key(call)?, true).await,
        Operation::DeleteObject => objects::delete(call, objects::key(call)?).await,
        _ => Err(Error::new(
            ErrorCode::InternalError,
            "the operation has no handler",
        )),
    }
}
