//! The decision: deny by default, explicit denies first, then the one rule that may allow.

use super::model::{Access, Action, BucketResource, Principal, Role, UserPrincipal};

/// Why a request was refused — for the decision log and for tests; a caller is told only that it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// The bucket or space belongs to another space: answered as if it did not exist.
    OtherSpace,
    /// The role does not include the action.
    Role,
    /// No grant on the bucket covers the action.
    NoGrant,
}

/// The evaluator's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The action may proceed.
    Allow,
    /// It may not, and why.
    Deny(Denied),
}

/// Decides whether `principal` may perform `action`. Pure: everything it reads is in its arguments.
#[must_use]
pub fn authorize(principal: &Principal, action: &Action) -> Decision {
    let user = match principal {
        Principal::Root => return Decision::Allow,
        Principal::User(user) => user,
    };
    if user.operator {
        return Decision::Allow;
    }
    match action {
        Action::ListBuckets => Decision::Allow,
        Action::ViewCluster => allow_if(user.cluster_viewer, Denied::Role),
        Action::Operate => Decision::Deny(Denied::Role),
        Action::ManageSpace(space) => {
            if *space != user.space {
                Decision::Deny(Denied::OtherSpace)
            } else {
                allow_if(user.role == Role::SpaceAdmin, Denied::Role)
            }
        }
        Action::CreateBucket(space) => {
            if *space != user.space {
                Decision::Deny(Denied::OtherSpace)
            } else {
                allow_if(
                    user.role == Role::SpaceAdmin || user.create_buckets,
                    Denied::Role,
                )
            }
        }
        Action::DeleteBucket(bucket) => on_bucket(user, bucket, |_| false),
        Action::ReadBucket(bucket) | Action::ReadObject(bucket) => {
            on_bucket(user, bucket, |access| access.read)
        }
        Action::WriteObject(bucket) => on_bucket(user, bucket, |access| access.write),
    }
}

const fn allow_if(allowed: bool, otherwise: Denied) -> Decision {
    if allowed {
        Decision::Allow
    } else {
        Decision::Deny(otherwise)
    }
}

/// A bucket action: its space's administrator and its creator may; a member otherwise needs a grant `covers`.
fn on_bucket(
    user: &UserPrincipal,
    bucket: &BucketResource,
    covers: impl Fn(Access) -> bool,
) -> Decision {
    if bucket.space != user.space {
        return Decision::Deny(Denied::OtherSpace);
    }
    if user.role == Role::SpaceAdmin || bucket.creator.as_deref() == Some(user.name.as_str()) {
        return Decision::Allow;
    }
    let granted = user.grants.get(&bucket.name).copied().unwrap_or_default();
    allow_if(covers(granted), Denied::NoGrant)
}
