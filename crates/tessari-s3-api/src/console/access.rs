//! Deciding a console request: the principal the session guard resolved, asked of the evaluator. Root skips the
//! lookups; a user's request loads the bucket it names, and one it cannot reach answers alike whether it exists or not.

use tessari_s3_constants::CONSOLE_FRESH_AUTH_SECS;
use tessari_s3_core::authz::policy::Context;
use tessari_s3_core::authz::{
    Action, Asked, BucketResource, Decision, Principal, authorize, authorize_asked,
};
use tessari_s3_core::console::Session;
use tessari_s3_types::BucketName;

use super::ConsoleState;
use super::error::ConsoleError;

/// Who is asking and what their request brings to a bucket policy, as one extension for a handler with many inputs.
#[derive(Debug, Clone)]
pub(super) struct Caller {
    /// The principal the session resolved.
    pub(super) principal: Principal,
    /// The request's facts for policy conditions.
    pub(super) context: Context,
}

/// Refuses a privileged act unless the session's holder authenticated within `CONSOLE_FRESH_AUTH_SECS`; checked
/// before anything the request carries is read.
pub(super) fn recently_authenticated(
    state: &ConsoleState,
    session: &Session,
) -> Result<(), ConsoleError> {
    if session.authenticated_within(state.now(), CONSOLE_FRESH_AUTH_SECS) {
        Ok(())
    } else {
        Err(ConsoleError::reauthentication_required())
    }
}

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

/// Refuses unless `principal` may perform the action `make` builds on bucket `name`, asked of the bucket's policy as
/// `asked` (the S3 action and the key) — the same question the S3 router asks, so a policy's Deny binds the console
/// too. A setting only the console has is asked under an `s3:` name of its own (`s3:PutBucketMedia`), which no policy
/// may name but `s3:*` and `s3:Put*` reach.
pub(super) async fn on_bucket(
    state: &ConsoleState,
    principal: &Principal,
    context: &Context,
    name: &BucketName,
    make: fn(BucketResource) -> Action,
    asked: (&'static str, Option<&str>),
) -> Result<(), ConsoleError> {
    let found = state.storage().buckets().get(name).await?;
    // A principal that may not reach the bucket is refused alike whether it exists or not, before its region is said.
    if !matches!(principal, Principal::Root) {
        let bucket = found.as_ref().ok_or(ConsoleError::forbidden())?;
        let (action, key) = asked;
        let question = Asked {
            action,
            key,
            context,
        };
        match authorize_asked(principal, &make(bucket.resource()), &question) {
            Decision::Allow => {}
            Decision::Deny(reason) => {
                tracing::info!(?reason, "console request denied");
                return Err(ConsoleError::forbidden());
            }
        }
    }
    match found {
        // Its objects and its lifecycle work are in that region's database: this node's would answer empty.
        Some(bucket) if bucket.region != state.region() => Err(ConsoleError::conflict(
            "other_region",
            "this bucket lives in another region; open the console of a node in that region",
        )),
        _ => Ok(()),
    }
}
