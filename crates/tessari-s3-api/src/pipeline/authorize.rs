//! Authorization: every implemented operation, for every principal, is judged by the one evaluator before its
//! handler runs, asked of the bucket's policy as the operation's S3 action on the key it addresses (S3 ref 04). Root
//! may do everything without a lookup; anyone else's request loads the bucket it addresses (and a copy's source
//! bucket). A batch delete is asked key by key in its handler, so this step only resolves who may reach the bucket.

use axum::http::HeaderMap;
use tessari_s3_core::authz::Need;
use tessari_s3_core::authz::policy::{Context, policy_action};
use tessari_s3_core::authz::{
    Action, Asked, BucketResource, Decision, Principal, authorize, authorize_asked, required,
    required_naming_version,
};
use tessari_s3_core::dispatch::Operation;
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

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
    names_version: bool,
    context: &Context,
) -> Result<()> {
    if matches!(principal, Principal::Root) {
        return Ok(());
    }
    let needs = if names_version {
        required_naming_version(operation)
    } else {
        required(operation)
    }
    .ok_or_else(denied)?;
    // A batch delete on a bucket with a policy, or by a session with one, is asked key by key by its handler
    // (`may_delete`); otherwise the bucket's grant decides every key at once, as it always has.
    if operation == Operation::DeleteObjects {
        let resource = bucket(state, addressed.bucket.as_ref()).await?;
        let narrowed =
            matches!(principal, Principal::Session(session) if session.policy().is_some());
        if resource.policy.is_some() || narrowed {
            return Ok(());
        }
        return allowed(principal, &Action::WriteObject(resource), None);
    }
    let key = addressed.key.as_ref().map(ObjectKey::as_str);
    let asked_action = policy_action(operation, names_version).map(|(action, _)| action);
    let asked = |action: &'static str| Asked {
        action,
        key,
        context,
    };
    // A one-key credential asks to upload the key it addresses; anything else it asks is judged as asked, and refused.
    if let (Principal::Upload(_), true, Some(key)) = (principal, needs.upload, key) {
        let action = Action::Upload {
            bucket: bucket(state, addressed.bucket.as_ref()).await?,
            key: key.to_owned(),
        };
        return allowed(principal, &action, asked_action.map(asked).as_ref());
    }
    let action = match needs.primary {
        Need::ListBuckets => Action::ListBuckets,
        Need::CreateBucket => Action::CreateBucket(home(principal).ok_or_else(denied)?),
        Need::DeleteBucket => Action::DeleteBucket(bucket(state, addressed.bucket.as_ref()).await?),
        Need::ConfigureBucket => {
            Action::ConfigureBucket(bucket(state, addressed.bucket.as_ref()).await?)
        }
        Need::ReadBucket => Action::ReadBucket(bucket(state, addressed.bucket.as_ref()).await?),
        Need::ReadObject => Action::ReadObject(bucket(state, addressed.bucket.as_ref()).await?),
        Need::WriteObject => Action::WriteObject(bucket(state, addressed.bucket.as_ref()).await?),
    };
    allowed(principal, &action, asked_action.map(asked).as_ref())?;
    if needs.source_read {
        let (source, source_key) = source_of(headers)?;
        let read = Action::ReadObject(bucket(state, Some(&source)).await?);
        let asked = Asked {
            action: "s3:GetObject",
            key: Some(source_key.as_str()),
            context,
        };
        allowed(principal, &read, Some(&asked))?;
    }
    Ok(())
}

/// The space a principal creates buckets in; none for one that creates nothing.
fn home(principal: &Principal) -> Option<tessari_s3_core::authz::SpaceName> {
    match principal {
        Principal::User(user) => Some(user.space.clone()),
        Principal::Upload(upload) => Some(upload.parent.space.clone()),
        Principal::Session(session) => home(session.as_user()),
        Principal::Root | Principal::Anonymous => None,
    }
}

/// Which of `entries` (key, whether it names a version) `principal` may delete from bucket `name`, each asked of the
/// bucket's policy for its own key — `s3:DeleteObjectVersion` for a named version, which also needs who administers
/// the bucket — against one read of the bucket.
///
/// # Errors
/// The store's failure when the bucket cannot be read.
pub async fn may_delete(
    state: &ApiState,
    principal: &Principal,
    name: &BucketName,
    entries: &[(&ObjectKey, bool)],
    context: &Context,
) -> Result<Vec<bool>> {
    if matches!(principal, Principal::Root) {
        return Ok(vec![true; entries.len()]);
    }
    let Some(resource) = reachable(state, name).await? else {
        return Ok(vec![false; entries.len()]);
    };
    let (configure, write) = (
        Action::ConfigureBucket(resource.clone()),
        Action::WriteObject(resource),
    );
    Ok(entries
        .iter()
        .map(|(key, names_version)| {
            let (action, asked) = if *names_version {
                (&configure, "s3:DeleteObjectVersion")
            } else {
                (&write, "s3:DeleteObject")
            };
            let asked = Asked {
                action: asked,
                key: Some(key.as_str()),
                context,
            };
            authorize_asked(principal, action, &asked) == Decision::Allow
        })
        .collect())
}

/// Whether `principal` may bypass governance retention on `key` of bucket `name`: who administers the bucket, asked
/// as `s3:BypassGovernanceRetention`.
///
/// # Errors
/// `AccessDenied` when the bucket cannot be reached; the store's failure when it cannot be read.
pub async fn may_bypass(
    state: &ApiState,
    principal: &Principal,
    name: &BucketName,
    key: &ObjectKey,
    context: &Context,
) -> Result<bool> {
    if matches!(principal, Principal::Root) {
        return Ok(true);
    }
    let action = Action::ConfigureBucket(bucket(state, Some(name)).await?);
    let asked = Asked {
        action: "s3:BypassGovernanceRetention",
        key: Some(key.as_str()),
        context,
    };
    Ok(authorize_asked(principal, &action, &asked) == Decision::Allow)
}

/// Whether `principal` may list bucket `name` — what telling a missing key apart from a forbidden one needs.
///
/// # Errors
/// The store's failure when the bucket cannot be read.
pub async fn may_list(
    state: &ApiState,
    principal: &Principal,
    name: &BucketName,
    context: &Context,
) -> Result<bool> {
    if matches!(principal, Principal::Root) {
        return Ok(true);
    }
    let Some(resource) = reachable(state, name).await? else {
        return Ok(false);
    };
    let asked = Asked {
        action: "s3:ListBucket",
        key: None,
        context,
    };
    Ok(authorize_asked(principal, &Action::ReadBucket(resource), &asked) == Decision::Allow)
}

/// The bucket `name` as the evaluator needs it, or `None` when there is none.
async fn reachable(state: &ApiState, name: &BucketName) -> Result<Option<BucketResource>> {
    Ok(state
        .storage()
        .buckets()
        .get(name)
        .await?
        .map(|bucket| bucket.resource()))
}

/// The bucket `name` as the evaluator needs it; a missing bucket is refused like one the caller may not reach.
async fn bucket(state: &ApiState, name: Option<&BucketName>) -> Result<BucketResource> {
    let name = name.ok_or_else(denied)?;
    reachable(state, name).await?.ok_or_else(denied)
}

fn allowed(principal: &Principal, action: &Action, asked: Option<&Asked<'_>>) -> Result<()> {
    let decision = match asked {
        Some(asked) => authorize_asked(principal, action, asked),
        None => authorize(principal, action),
    };
    match decision {
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
