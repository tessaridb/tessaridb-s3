//! Deciding a console request: the principal the session guard resolved, asked of the evaluator. Root skips the
//! lookups; a user's request loads the bucket it names, and one it cannot reach answers alike whether it exists or not.

use tessari_s3_core::authz::{Action, BucketResource, Decision, Principal, authorize};
use tessari_s3_types::BucketName;

use super::ConsoleState;
use super::error::ConsoleError;

/// Refuses unless `principal` may perform `action`.
pub(super) fn allow(principal: &Principal, action: &Action) -> Result<(), ConsoleError> {
    match authorize(principal, action) {
        Decision::Allow => Ok(()),
        Decision::Deny(reason) => {
            tracing::info!(?reason, "console request denied");
            Err(ConsoleError::forbidden())
        }
    }
}

/// Refuses unless `principal` may perform the action `make` builds on bucket `name`.
pub(super) async fn on_bucket(
    state: &ConsoleState,
    principal: &Principal,
    name: &BucketName,
    make: fn(BucketResource) -> Action,
) -> Result<(), ConsoleError> {
    if matches!(principal, Principal::Root) {
        return Ok(());
    }
    let bucket = state
        .storage()
        .buckets()
        .get(name)
        .await?
        .ok_or(ConsoleError::forbidden())?;
    allow(principal, &make(bucket.resource()))
}
