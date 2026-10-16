//! The operations with handlers, one module per domain. An operation marked implemented in the core's support
//! table but missing here is refused as an internal error rather than routed anywhere else.

use axum::body::Body;
use axum::http::Response;
use tessari_s3_core::dispatch::Operation;
use tessari_s3_types::ErrorCode;

use crate::pipeline::call::Call;
use crate::{Error, Result};

pub mod buckets;
mod encryption;
mod lifecycle;
pub mod listing;
pub mod multipart;
mod notification;
mod object_lock;
pub mod objects;
mod policy;
mod quota;
mod replication;
mod retention;
pub(crate) mod tagging;
mod versioning;
mod xml_tree;

/// Runs the handler for `operation`.
///
/// # Errors
/// The handler's S3 error, or `InternalError` for an implemented operation with no handler.
pub(crate) async fn route(
    operation: Operation,
    call: &Call<'_>,
    body: Body,
) -> Result<Response<Body>> {
    // A bucket's configuration is in the global database, so these would answer anywhere: they are sent to the
    // bucket's region like its objects are (object paths are sent there by the storage layer's own check). The
    // operations carrying a body check it in their handler once the body has parsed, and a batch delete is checked
    // once there rather than answering each key with an error.
    if matches!(
        operation,
        Operation::HeadBucket
            | Operation::DeleteBucket
            | Operation::GetBucketVersioning
            | Operation::GetBucketEncryption
            | Operation::DeleteBucketEncryption
            | Operation::GetBucketLifecycleConfiguration
            | Operation::DeleteBucketLifecycle
            | Operation::GetBucketNotificationConfiguration
            | Operation::GetBucketReplication
            | Operation::DeleteBucketReplication
            | Operation::GetObjectLockConfiguration
            | Operation::GetBucketTagging
            | Operation::DeleteBucketTagging
            | Operation::GetBucketPolicy
            | Operation::DeleteBucketPolicy
    ) {
        buckets::resident(call).await?;
    }
    match operation {
        Operation::CreateBucket => buckets::create(call, body).await,
        Operation::HeadBucket => buckets::head(call).await,
        Operation::DeleteBucket => buckets::delete(call).await,
        Operation::ListBuckets => buckets::list(call).await,
        Operation::GetBucketLocation => buckets::location(call).await,
        Operation::GetBucketVersioning => versioning::get(call).await,
        Operation::PutBucketVersioning => versioning::put(call, body).await,
        Operation::GetBucketEncryption => encryption::get(call).await,
        Operation::PutBucketEncryption => encryption::put(call, body).await,
        Operation::DeleteBucketEncryption => encryption::delete(call).await,
        Operation::GetBucketLifecycleConfiguration => lifecycle::get(call).await,
        Operation::PutBucketLifecycleConfiguration => lifecycle::put(call, body).await,
        Operation::DeleteBucketLifecycle => lifecycle::delete(call).await,
        Operation::GetBucketReplication => replication::get(call).await,
        Operation::PutBucketReplication => replication::put(call, body).await,
        Operation::DeleteBucketReplication => replication::delete(call).await,
        Operation::GetBucketNotificationConfiguration => notification::get(call).await,
        Operation::PutBucketNotificationConfiguration => notification::put(call, body).await,
        Operation::ListObjects => listing::list_v1(call).await,
        Operation::ListObjectsV2 => listing::list_v2(call).await,
        Operation::ListObjectVersions => listing::list_versions(call).await,
        Operation::PutObject => objects::put(call, objects::key(call)?, body).await,
        Operation::CopyObject => objects::copy(call, objects::key(call)?).await,
        Operation::GetObject => objects::read(call, objects::key(call)?, false).await,
        Operation::HeadObject => objects::read(call, objects::key(call)?, true).await,
        Operation::GetObjectTagging => tagging::get_object(call, objects::key(call)?).await,
        Operation::PutObjectTagging => tagging::put_object(call, objects::key(call)?, body).await,
        Operation::DeleteObjectTagging => tagging::delete_object(call, objects::key(call)?).await,
        Operation::GetBucketTagging => tagging::get_bucket(call).await,
        Operation::PutBucketTagging => tagging::put_bucket(call, body).await,
        Operation::DeleteBucketTagging => tagging::delete_bucket(call).await,
        Operation::GetBucketPolicy => policy::get(call).await,
        Operation::PutBucketPolicy => policy::put(call, body).await,
        Operation::DeleteBucketPolicy => policy::delete(call).await,
        Operation::GetObjectLockConfiguration => object_lock::get(call).await,
        Operation::PutObjectLockConfiguration => object_lock::put(call, body).await,
        Operation::GetObjectRetention => retention::get_retention(call, objects::key(call)?).await,
        Operation::PutObjectRetention => {
            retention::put_retention(call, objects::key(call)?, body).await
        }
        Operation::GetObjectLegalHold => retention::get_legal_hold(call, objects::key(call)?).await,
        Operation::PutObjectLegalHold => {
            retention::put_legal_hold(call, objects::key(call)?, body).await
        }
        Operation::DeleteObject => objects::delete(call, objects::key(call)?).await,
        Operation::DeleteObjects => objects::delete_many(call, body).await,
        Operation::CreateMultipartUpload => multipart::create(call, objects::key(call)?).await,
        Operation::UploadPart => multipart::upload_part(call, objects::key(call)?, body).await,
        Operation::UploadPartCopy => multipart::copy_part(call, objects::key(call)?).await,
        Operation::AbortMultipartUpload => multipart::abort(call, objects::key(call)?).await,
        Operation::ListParts => multipart::list_parts(call, objects::key(call)?).await,
        Operation::ListMultipartUploads => multipart::list_uploads(call).await,
        Operation::CompleteMultipartUpload => {
            multipart::complete(call, objects::key(call)?, body).await
        }
        _ => Err(Error::new(
            ErrorCode::InternalError,
            "the operation has no handler",
        )),
    }
}
