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
