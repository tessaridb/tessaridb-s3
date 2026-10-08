//! Authorization: every implemented operation, for every principal, is judged by the one evaluator before its
//! handler runs. The root credential may do everything without a lookup; a user's request loads the bucket it
//! addresses (and a copy's source bucket) and is decided against it. Grants are per bucket, so the bucket's decision
//! for `DeleteObjects` is the decision for every key the batch names. A one-key upload credential's step of uploading
//! is judged with the key it addresses, and every other request it signs is judged as asked and refused.

use axum::http::HeaderMap;
use tessari_s3_core::authz::Need;
use tessari_s3_core::authz::{Action, BucketResource, Decision, Principal, authorize, required};
use tessari_s3_core::dispatch::Operation;
use tessari_s3_types::{BucketName, ErrorCode};

use super::address::Addressed;
use crate::routes::objects::source::source_of;
use crate::state::ApiState;
use crate::{Error, Result};

/// Refuses the request unless `principal` may perform `operation` on what it addresses.
///
/// A bucket the caller cannot reach answers `AccessDenied` whether it exists or not, so names cannot be probed across
/// spaces.
///
/// # Errors
/// `AccessDenied` when the evaluator denies; the store's failure when a bucket cannot be read.
pub async fn authorize_operation(
    state: &ApiState,
    operation: Operation,
    principal: &Principal,
    addressed: &Addressed,
    headers: &HeaderMap,
) -> Result<()> {
    let space = match principal {
        Principal::Root => return Ok(()),
        Principal::User(user) => &user.space,
        Principal::Upload(upload) => &upload.parent.space,
    };
    let needs = required(operation).ok_or_else(denied)?;
    // A one-key credential asks to upload the key it addresses; anything else it asks is judged as asked, and refused.
    if let (Principal::Upload(_), true, Some(key)) = (principal, needs.upload, &addressed.key) {
        let action = Action::Upload {
            bucket: bucket(state, addressed.bucket.as_ref()).await?,
            key: key.as_str().to_owned(),
        };
        return allowed(principal, &action);
    }
    let action = match needs.primary {
        Need::ListBuckets => Action::ListBuckets,
        Need::CreateBucket => Action::CreateBucket(space.clone()),
        Need::DeleteBucket => Action::DeleteBucket(bucket(state, addressed.bucket.as_ref()).await?),
        Need::ReadBucket => Action::ReadBucket(bucket(state, addressed.bucket.as_ref()).await?),
        Need::ReadObject => Action::ReadObject(bucket(state, addressed.bucket.as_ref()).await?),
        Need::WriteObject => Action::WriteObject(bucket(state, addressed.bucket.as_ref()).await?),
    };
    allowed(principal, &action)?;
    if needs.source_read {
        let (source, _) = source_of(headers)?;
        allowed(
            principal,
            &Action::ReadObject(bucket(state, Some(&source)).await?),
        )?;
    }
    Ok(())
}

/// The bucket `name` as the evaluator needs it; a missing bucket is refused like one the caller may not reach.
async fn bucket(state: &ApiState, name: Option<&BucketName>) -> Result<BucketResource> {
    let name = name.ok_or_else(denied)?;
    let bucket = state
        .storage()
        .buckets()
        .get(name)
        .await?
        .ok_or_else(denied)?;
    Ok(bucket.resource())
}

fn allowed(principal: &Principal, action: &Action) -> Result<()> {
    match authorize(principal, action) {
        Decision::Allow => Ok(()),
        Decision::Deny(reason) => {
            tracing::info!(?reason, "access denied");
            Err(denied())
        }
    }
}

fn denied() -> Error {
    Error::new(ErrorCode::AccessDenied, "Access Denied")
}
