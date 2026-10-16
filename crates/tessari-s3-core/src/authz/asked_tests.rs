//! The one decision with a bucket's policy in it (S3 ref 04 M1): Root first; then a policy's explicit Deny, which no
//! role or grant outweighs; then the caller's own grants; then a policy's Allow — never widening a one-key credential.
//! A policy-bearing bucket asked about without the policy question is refused, so no path can skip the policy.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::policy::{BucketPolicy, Context};
use super::{
    Action, Asked, BucketResource, Decision, Denied, Principal, Role, SpaceName, UploadPrincipal,
    UserPrincipal, authorize, authorize_asked,
};

fn space(name: &str) -> SpaceName {
    SpaceName::new(name).expect("space")
}

fn user(name: &str, in_space: &str, role: Role) -> UserPrincipal {
    UserPrincipal {
        name: name.to_owned(),
        space: space(in_space),
        role,
        create_buckets: false,
        operator: false,
        cluster_viewer: false,
        grants: BTreeMap::new(),
    }
}

fn governed(statements: &str) -> BucketResource {
    let text = format!(r#"{{"Version":"2012-10-17","Statement":[{statements}]}}"#);
    BucketResource {
        name: "photos".to_owned(),
        space: space("acme"),
        creator: Some("ann".to_owned()),
        policy: Some(Arc::new(BucketPolicy::parse(
            &text,
            "photos",
            &space("acme"),
        ))),
    }
}

fn ask<'a>(action: &'a str, key: Option<&'a str>, context: &'a Context) -> Asked<'a> {
    Asked {
        action,
        key,
        context,
    }
}

const PUBLIC_READ: &str = r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#;
const DENY_SECRET: &str = r#"{"Effect":"Deny","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::photos/secret/*"}"#;

#[test]
fn an_anonymous_read_is_allowed_only_by_a_public_statement() {
    let c = Context::default();
    let bucket = governed(PUBLIC_READ);
    let read = Action::ReadObject(bucket.clone());
    assert_eq!(
        authorize_asked(
            &Principal::Anonymous,
            &read,
            &ask("s3:GetObject", Some("a"), &c)
        ),
        Decision::Allow
    );
    let write = Action::WriteObject(bucket);
    assert_eq!(
        authorize_asked(
            &Principal::Anonymous,
            &write,
            &ask("s3:PutObject", Some("a"), &c)
        ),
        Decision::Deny(Denied::NoGrant)
    );
    let ungoverned = BucketResource {
        policy: None,
        ..governed(PUBLIC_READ)
    };
    assert_eq!(
        authorize_asked(
            &Principal::Anonymous,
            &Action::ReadObject(ungoverned),
            &ask("s3:GetObject", Some("a"), &c)
        ),
        Decision::Deny(Denied::NoGrant)
    );
}

#[test]
fn an_explicit_deny_outweighs_the_space_admin_the_creator_and_an_operator() {
    let c = Context::default();
    let action = Action::ReadObject(governed(&format!("{PUBLIC_READ},{DENY_SECRET}")));
    let mut operator = user("op", "acme", Role::Member);
    operator.operator = true;
    for principal in [
        Principal::User(user("boss", "acme", Role::SpaceAdmin)),
        Principal::User(user("ann", "acme", Role::Member)),
        Principal::User(operator),
    ] {
        assert_eq!(
            authorize_asked(
                &principal,
                &action,
                &ask("s3:GetObject", Some("secret/x"), &c)
            ),
            Decision::Deny(Denied::Policy),
            "{principal:?}"
        );
        assert_eq!(
            authorize_asked(
                &principal,
                &action,
                &ask("s3:GetObject", Some("open/x"), &c)
            ),
            Decision::Allow
        );
    }
    assert_eq!(
        authorize_asked(
            &Principal::Root,
            &action,
            &ask("s3:GetObject", Some("secret/x"), &c)
        ),
        Decision::Allow,
        "Root is the documented exception"
    );
}

#[test]
fn a_user_of_another_space_is_reached_by_a_public_statement_only() {
    let c = Context::default();
    let stranger = Principal::User(user("eve", "other", Role::SpaceAdmin));
    let public = Action::ReadObject(governed(PUBLIC_READ));
    assert_eq!(
        authorize_asked(&stranger, &public, &ask("s3:GetObject", Some("a"), &c)),
        Decision::Allow
    );
    let named = Action::ReadObject(governed(
        r#"{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::acme:root"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#,
    ));
    assert_eq!(
        authorize_asked(&stranger, &named, &ask("s3:GetObject", Some("a"), &c)),
        Decision::Deny(Denied::OtherSpace)
    );
}

#[test]
fn a_member_without_a_grant_is_reached_by_a_statement_naming_them() {
    let c = Context::default();
    let bob = Principal::User(user("bob", "acme", Role::Member));
    let action = Action::WriteObject(governed(
        r#"{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::acme:user/bob"},"Action":"s3:PutObject","Resource":"arn:aws:s3:::photos/inbox/*"}"#,
    ));
    assert_eq!(
        authorize_asked(&bob, &action, &ask("s3:PutObject", Some("inbox/1"), &c)),
        Decision::Allow
    );
    assert_eq!(
        authorize_asked(&bob, &action, &ask("s3:PutObject", Some("other/1"), &c)),
        Decision::Deny(Denied::NoGrant)
    );
}

#[test]
fn a_policy_never_widens_a_one_key_credential_and_its_deny_still_binds_it() {
    let c = Context::default();
    let upload = Principal::Upload(UploadPrincipal {
        parent: user("ann", "acme", Role::Member),
        bucket: "photos".to_owned(),
        key: "inbox/1".to_owned(),
    });
    let open = governed(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::photos/*"}"#,
    );
    let other_key = Action::Upload {
        bucket: open.clone(),
        key: "inbox/2".to_owned(),
    };
    assert_eq!(
        authorize_asked(
            &upload,
            &other_key,
            &ask("s3:PutObject", Some("inbox/2"), &c)
        ),
        Decision::Deny(Denied::OutOfScope)
    );
    let its_key = Action::Upload {
        bucket: open,
        key: "inbox/1".to_owned(),
    };
    assert_eq!(
        authorize_asked(&upload, &its_key, &ask("s3:PutObject", Some("inbox/1"), &c)),
        Decision::Allow
    );
    let denied = governed(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:PutObject","Resource":"arn:aws:s3:::photos/inbox/*"}"#,
    );
    let blocked = Action::Upload {
        bucket: denied,
        key: "inbox/1".to_owned(),
    };
    assert_eq!(
        authorize_asked(&upload, &blocked, &ask("s3:PutObject", Some("inbox/1"), &c)),
        Decision::Deny(Denied::Policy)
    );
}

#[test]
fn a_governed_bucket_asked_without_the_policy_question_is_refused() {
    let admin = Principal::User(user("boss", "acme", Role::SpaceAdmin));
    let action = Action::ReadObject(governed(PUBLIC_READ));
    assert_eq!(authorize(&admin, &action), Decision::Deny(Denied::Unasked));
    assert_eq!(authorize(&Principal::Root, &action), Decision::Allow);
}

#[test]
fn a_policy_that_no_longer_parses_refuses_everyone_but_root() {
    let c = Context::default();
    let broken = BucketResource {
        policy: Some(Arc::new(BucketPolicy::parse("{", "photos", &space("acme")))),
        ..governed(PUBLIC_READ)
    };
    let admin = Principal::User(user("boss", "acme", Role::SpaceAdmin));
    let action = Action::ConfigureBucket(broken);
    assert_eq!(
        authorize_asked(&admin, &action, &ask("s3:PutBucketPolicy", None, &c)),
        Decision::Deny(Denied::PolicyUnreadable)
    );
    assert_eq!(
        authorize_asked(
            &Principal::Root,
            &action,
            &ask("s3:PutBucketPolicy", None, &c)
        ),
        Decision::Allow
    );
}
