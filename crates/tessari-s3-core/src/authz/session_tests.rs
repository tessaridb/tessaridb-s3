//! Temporary credentials: a session's authority is its user's — every decision and every listing the user would get,
//! and nothing the user would not.

use std::collections::BTreeMap;

use super::{
    Access, Action, BucketResource, Decision, Denied, Principal, Role, SessionPrincipal, SpaceName,
    UserPrincipal, authorize,
};

fn space(name: &str) -> SpaceName {
    SpaceName::new(name).expect("a valid space name")
}

fn ann() -> UserPrincipal {
    let mut grants = BTreeMap::new();
    grants.insert(
        "shared".to_owned(),
        Access {
            read: true,
            write: false,
        },
    );
    UserPrincipal {
        name: "ann".to_owned(),
        space: space("alpha"),
        role: Role::Member,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
        grants,
    }
}

fn bucket(name: &str, in_space: &str) -> BucketResource {
    BucketResource {
        name: name.to_owned(),
        space: space(in_space),
        creator: Some("owner".to_owned()),
        policy: None,
        public_access: super::policy::PublicAccessBlock::default(),
    }
}

#[test]
fn a_session_decides_as_its_user_and_no_wider() {
    let session = Principal::Session(SessionPrincipal::new(ann()));
    let user = Principal::User(ann());
    let actions = [
        Action::ReadObject(bucket("shared", "alpha")),
        Action::WriteObject(bucket("shared", "alpha")),
        Action::ReadObject(bucket("other", "alpha")),
        Action::ReadObject(bucket("shared", "beta")),
        Action::ManageSpace(space("alpha")),
        Action::CreateBucket(space("alpha")),
        Action::ListBuckets,
        Action::Operate,
    ];
    for action in &actions {
        assert_eq!(
            authorize(&session, action),
            authorize(&user, action),
            "{action:?}"
        );
    }
    assert_eq!(
        authorize(&session, &Action::ReadObject(bucket("shared", "alpha"))),
        Decision::Allow
    );
    assert_eq!(
        authorize(&session, &Action::WriteObject(bucket("shared", "alpha"))),
        Decision::Deny(Denied::NoGrant)
    );
    assert_eq!(
        session.visible(),
        user.visible(),
        "it lists what its user lists"
    );
}

fn narrowed(policy: &str) -> Principal {
    let mut user = ann();
    user.grants.insert(
        "shared".to_owned(),
        Access {
            read: true,
            write: true,
        },
    );
    let text = format!(r#"{{"Version":"2012-10-17","Statement":[{policy}]}}"#);
    let policy = super::policy::SessionPolicy::parse(&text).expect("a session policy");
    Principal::Session(SessionPrincipal::narrowed(
        user,
        std::sync::Arc::new(policy),
    ))
}

fn asked(principal: &Principal, action: &Action, s3_action: &str, key: Option<&str>) -> Decision {
    let context = super::policy::Context::default();
    super::authorize_asked(
        principal,
        action,
        &super::Asked {
            action: s3_action,
            key,
            context: &context,
        },
    )
}

#[test]
fn a_session_policy_narrows_its_user_and_never_widens_it() {
    let reader = narrowed(
        r#"{"Effect":"Allow","Action":"s3:GetObject","Resource":"arn:aws:s3:::shared/*"},
           {"Effect":"Allow","Action":"s3:*","Resource":"arn:aws:s3:::other/*"}"#,
    );
    let shared = || bucket("shared", "alpha");
    assert_eq!(
        asked(
            &reader,
            &Action::ReadObject(shared()),
            "s3:GetObject",
            Some("a")
        ),
        Decision::Allow
    );
    assert_eq!(
        asked(
            &reader,
            &Action::WriteObject(shared()),
            "s3:PutObject",
            Some("a")
        ),
        Decision::Deny(Denied::SessionPolicy),
        "the user may write; the session policy does not allow it"
    );
    assert_eq!(
        asked(
            &reader,
            &Action::ReadObject(bucket("other", "alpha")),
            "s3:GetObject",
            Some("a")
        ),
        Decision::Deny(Denied::NoGrant),
        "a policy allowing more than the user grants nothing more"
    );
    assert_eq!(
        authorize(&reader, &Action::ReadObject(shared())),
        Decision::Deny(Denied::SessionPolicy),
        "a narrowed session is not decided without the policy question"
    );
    let fenced = narrowed(
        r#"{"Effect":"Allow","Action":"s3:*","Resource":"*"},
           {"Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::shared/private/*"}"#,
    );
    assert_eq!(
        asked(
            &fenced,
            &Action::ReadObject(shared()),
            "s3:GetObject",
            Some("private/k")
        ),
        Decision::Deny(Denied::SessionPolicy),
        "a Deny in the session policy outweighs the user's grant"
    );
    assert_eq!(
        asked(
            &fenced,
            &Action::ReadObject(shared()),
            "s3:GetObject",
            Some("public/k")
        ),
        Decision::Allow
    );
}
