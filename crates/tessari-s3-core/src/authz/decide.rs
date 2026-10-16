//! The decision: deny by default, explicit denies first, then the one rule that may allow. A bucket's policy joins
//! it (S3 ref 04 M1): its explicit Deny outweighs every role and grant, the caller's own grants come next, and its
//! Allow last — the IAM order, with Root the documented exception.

use super::model::{
    Access, Action, BucketResource, Principal, Role, UploadPrincipal, UserPrincipal, UserResource,
};
use super::policy::{Context, PolicyAnswer, PolicyRequest, Requester};

/// Why a request was refused — for the decision log and for tests; a caller is told only that it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// The bucket or space belongs to another space: answered as if it did not exist.
    OtherSpace,
    /// The role does not include the action.
    Role,
    /// No grant on the bucket covers the action.
    NoGrant,
    /// The caller is the user it would manage.
    SelfManagement,
    /// A one-key credential asked for anything but uploading its key.
    OutOfScope,
    /// The bucket's policy explicitly denies it.
    Policy,
    /// The bucket's stored policy no longer parses: nothing it governs is decided until it is replaced.
    PolicyUnreadable,
    /// The bucket carries a policy and the caller asked without the policy question.
    Unasked,
    /// Only a public statement allows it, and RestrictPublicBuckets keeps those to the bucket's own space.
    Restricted,
}

/// The policy question a request asks: the S3 action, the key (none for the bucket itself) and its facts.
#[derive(Debug, Clone, Copy)]
pub struct Asked<'a> {
    /// The S3 action, `s3:GetObject`.
    pub action: &'a str,
    /// The object key; `None` asks about the bucket.
    pub key: Option<&'a str>,
    /// The request's facts for conditions.
    pub context: &'a Context,
}

/// The evaluator's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The action may proceed.
    Allow,
    /// It may not, and why.
    Deny(Denied),
}

/// Decides whether `principal` may perform `action` by roles and grants alone. A bucket that carries a policy is
/// refused here — only [`authorize_asked`] can decide it. Pure: everything it reads is in its arguments.
#[must_use]
pub fn authorize(principal: &Principal, action: &Action) -> Decision {
    decide(principal, action, None)
}

/// Decides whether `principal` may perform `action`, asked as `asked` of the bucket's policy when it has one.
#[must_use]
pub fn authorize_asked(principal: &Principal, action: &Action, asked: &Asked<'_>) -> Decision {
    decide(principal, action, Some(asked))
}

fn decide(principal: &Principal, action: &Action, asked: Option<&Asked<'_>>) -> Decision {
    if matches!(principal, Principal::Root) {
        return Decision::Allow;
    }
    let Some((bucket, stored)) = action
        .bucket()
        .and_then(|bucket| Some((bucket, bucket.policy.as_deref()?)))
    else {
        return identity(principal, action, asked);
    };
    let Some(asked) = asked else {
        return Decision::Deny(Denied::Unasked);
    };
    let Ok(policy) = stored else {
        return Decision::Deny(Denied::PolicyUnreadable);
    };
    let answer = policy.evaluate(&PolicyRequest {
        requester: requester(principal),
        action: asked.action,
        key: asked.key,
        context: asked.context,
    });
    match (answer, identity(principal, action, Some(asked))) {
        (PolicyAnswer::Deny { .. }, _) => Decision::Deny(Denied::Policy),
        (_, Decision::Allow) => Decision::Allow,
        // A one-key credential's authority is its issuer's narrowed to its key: a policy never widens it.
        (PolicyAnswer::Allow { .. }, denied) if matches!(principal, Principal::Upload(_)) => denied,
        // RestrictPublicBuckets: a public policy's Allows reach only the bucket's own space.
        (PolicyAnswer::Allow { .. }, _)
            if bucket.public_access.restrict_public_buckets
                && policy.is_public()
                && !matches!(principal, Principal::User(user) if user.space == bucket.space) =>
        {
            Decision::Deny(Denied::Restricted)
        }
        (PolicyAnswer::Allow { .. }, _) => Decision::Allow,
        (PolicyAnswer::NotApplicable, denied) => denied,
    }
}

/// Who asks, as a bucket policy sees them; a one-key credential is its issuer.
fn requester(principal: &Principal) -> Requester<'_> {
    match principal {
        Principal::User(user) => Requester::User {
            space: &user.space,
            name: &user.name,
        },
        Principal::Upload(upload) => Requester::User {
            space: &upload.parent.space,
            name: &upload.parent.name,
        },
        Principal::Root | Principal::Anonymous => Requester::Anonymous,
    }
}

/// Roles and grants: what the principal may do with no policy in the question.
fn identity(principal: &Principal, action: &Action, asked: Option<&Asked<'_>>) -> Decision {
    let user = match principal {
        Principal::Root => return Decision::Allow,
        Principal::User(user) => user,
        Principal::Upload(upload) => return within_scope(upload, action, asked),
        Principal::Anonymous => return Decision::Deny(Denied::NoGrant),
    };
    // Nobody manages their own user — not even an operator: a self-grant is refused by the system, never by policy.
    if let Action::ManageUser(target) = action
        && target.name == user.name
    {
        return Decision::Deny(Denied::SelfManagement);
    }
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
        Action::ManageUser(target) => manage_user(user, target),
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
        Action::DeleteBucket(bucket) | Action::ConfigureBucket(bucket) => {
            on_bucket(user, bucket, |_| false)
        }
        Action::ReadBucket(bucket) | Action::ReadObject(bucket) => {
            on_bucket(user, bucket, |access| access.read)
        }
        Action::WriteObject(bucket) | Action::Upload { bucket, .. } => {
            on_bucket(user, bucket, |access| access.write)
        }
    }
}

/// A one-key credential: uploading exactly its key of exactly its bucket, and then only what its issuer may still do
/// there — the intersection, judged against the issuer as they are now.
fn within_scope(upload: &UploadPrincipal, action: &Action, asked: Option<&Asked<'_>>) -> Decision {
    match action {
        Action::Upload { bucket, key } if bucket.name == upload.bucket && *key == upload.key => {
            decide(
                &Principal::User(upload.parent.clone()),
                &Action::WriteObject(bucket.clone()),
                asked,
            )
        }
        _ => Decision::Deny(Denied::OutOfScope),
    }
}

const fn allow_if(allowed: bool, otherwise: Denied) -> Decision {
    if allowed {
        Decision::Allow
    } else {
        Decision::Deny(otherwise)
    }
}

/// Managing a user: a space admin manages the plain members of its own space; authority equal to or above its own —
/// another admin, an operator, a cluster viewer — is the operators' to create or change, so delegation cannot widen.
fn manage_user(user: &UserPrincipal, target: &UserResource) -> Decision {
    if target.space != user.space {
        return Decision::Deny(Denied::OtherSpace);
    }
    let plain_member = target.role == Role::Member && !target.operator && !target.cluster_viewer;
    allow_if(user.role == Role::SpaceAdmin && plain_member, Denied::Role)
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
