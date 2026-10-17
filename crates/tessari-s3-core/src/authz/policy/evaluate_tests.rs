//! Evaluating a bucket policy: deny overrides allow, the principal, the action, the resource and every condition
//! must match, and a statement that does not apply says so rather than answering for the request.

use std::net::IpAddr;

use super::super::model::SpaceName;
use super::{BucketPolicy, Context, PolicyAnswer, PolicyRequest, Requester};

fn space() -> SpaceName {
    SpaceName::new("acme").expect("space")
}

fn policy(statements: &str) -> BucketPolicy {
    let text = format!(r#"{{"Version":"2012-10-17","Statement":[{statements}]}}"#);
    BucketPolicy::parse(&text, "photos", &space()).expect("accepted")
}

fn context() -> Context {
    Context {
        secure_transport: true,
        source_ip: "203.0.113.7".parse::<IpAddr>().ok(),
        now_secs: 1_800_000_000,
        ..Context::default()
    }
}

fn ask(
    policy: &BucketPolicy,
    requester: Requester<'_>,
    action: &str,
    key: Option<&str>,
    context: &Context,
) -> PolicyAnswer {
    policy.evaluate(&PolicyRequest {
        requester,
        action,
        key,
        context,
    })
}

fn ann(space: &SpaceName) -> Requester<'_> {
    Requester::User { space, name: "ann" }
}

const READ_ALL: &str = r#"{"Sid":"read","Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#;

#[test]
fn an_anonymous_read_is_allowed_by_a_public_statement_and_names_it() {
    let policy = policy(READ_ALL);
    let answer = ask(
        &policy,
        Requester::Anonymous,
        "s3:GetObject",
        Some("a.jpg"),
        &context(),
    );
    assert_eq!(
        answer,
        PolicyAnswer::Allow {
            sid: Some("read".to_owned())
        }
    );
}

#[test]
fn a_statement_answers_only_its_action_and_its_resource() {
    let policy = policy(READ_ALL);
    let c = context();
    assert_eq!(
        ask(
            &policy,
            Requester::Anonymous,
            "s3:PutObject",
            Some("a.jpg"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
    // The bucket's own ARN is not an object ARN: listing is not reading.
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:ListBucket", None, &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn an_explicit_deny_overrides_an_allow_whatever_the_order() {
    let deny = r#"{"Sid":"no","Effect":"Deny","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::photos/secret/*"}"#;
    for statements in [format!("{READ_ALL},{deny}"), format!("{deny},{READ_ALL}")] {
        let policy = policy(&statements);
        let c = context();
        assert_eq!(
            ask(
                &policy,
                Requester::Anonymous,
                "s3:GetObject",
                Some("secret/x"),
                &c
            ),
            PolicyAnswer::Deny {
                sid: Some("no".to_owned())
            }
        );
        assert!(matches!(
            ask(
                &policy,
                Requester::Anonymous,
                "s3:GetObject",
                Some("open/x"),
                &c
            ),
            PolicyAnswer::Allow { .. }
        ));
    }
}

#[test]
fn action_names_match_case_insensitively_and_by_wildcard() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"S3:get*","Resource":"arn:aws:s3:::photos/*"}"#,
    );
    let c = context();
    assert!(matches!(
        ask(
            &policy,
            Requester::Anonymous,
            "s3:GetObjectTagging",
            Some("k"),
            &c
        ),
        PolicyAnswer::Allow { .. }
    ));
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:PutObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn resource_patterns_match_byte_for_byte_with_star_and_question_mark() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/a?c/*.jpg"}"#,
    );
    let c = context();
    let allowed = |key| {
        matches!(
            ask(&policy, Requester::Anonymous, "s3:GetObject", Some(key), &c),
            PolicyAnswer::Allow { .. }
        )
    };
    assert!(allowed("abc/x/y.jpg"));
    assert!(!allowed("ac/x.jpg"));
    assert!(!allowed("ABC/x.jpg"));
    assert!(!allowed("abc/x.png"));
}

#[test]
fn not_action_and_not_resource_apply_to_everything_except_their_list() {
    let policy = policy(
        r#"{"Effect":"Deny","Principal":"*","NotAction":"s3:GetObject","NotResource":"arn:aws:s3:::photos/public/*"}"#,
    );
    let c = context();
    assert!(matches!(
        ask(
            &policy,
            Requester::Anonymous,
            "s3:PutObject",
            Some("private/k"),
            &c
        ),
        PolicyAnswer::Deny { .. }
    ));
    assert_eq!(
        ask(
            &policy,
            Requester::Anonymous,
            "s3:GetObject",
            Some("private/k"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
    assert_eq!(
        ask(
            &policy,
            Requester::Anonymous,
            "s3:PutObject",
            Some("public/k"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn a_named_principal_matches_that_user_only_and_never_the_anonymous_caller() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::acme:user/ann"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#,
    );
    let (acme, other) = (space(), SpaceName::new("other").expect("space"));
    let c = context();
    assert!(matches!(
        ask(&policy, ann(&acme), "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Allow { .. }
    ));
    assert_eq!(
        ask(
            &policy,
            Requester::User {
                space: &acme,
                name: "bob"
            },
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
    assert_eq!(
        ask(
            &policy,
            Requester::User {
                space: &other,
                name: "ann"
            },
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn the_space_root_principal_matches_every_user_of_the_space() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::acme:root"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}"#,
    );
    let acme = space();
    let c = context();
    assert!(matches!(
        ask(
            &policy,
            Requester::User {
                space: &acme,
                name: "bob"
            },
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::Allow { .. }
    ));
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn a_deny_unless_secure_transport_refuses_the_clear_and_spares_tls() {
    let policy = policy(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:*","Resource":"arn:aws:s3:::photos/*","Condition":{"Bool":{"aws:SecureTransport":"false"}}}"#,
    );
    let mut c = context();
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
    c.secure_transport = false;
    assert!(matches!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Deny { .. }
    ));
}

#[test]
fn source_ip_conditions_match_cidr_ranges_in_both_families() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"IpAddress":{"aws:SourceIp":["203.0.113.0/24","2001:db8::/32"]}}}"#,
    );
    let mut c = context();
    let allowed = |c: &Context| {
        matches!(
            ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), c),
            PolicyAnswer::Allow { .. }
        )
    };
    assert!(allowed(&c));
    c.source_ip = "198.51.100.1".parse().ok();
    assert!(!allowed(&c));
    c.source_ip = "2001:db8::9".parse().ok();
    assert!(allowed(&c));
    c.source_ip = None;
    assert!(
        !allowed(&c),
        "an absent key does not match a positive operator"
    );
}

#[test]
fn an_absent_key_fails_a_positive_operator_and_satisfies_a_negated_one_and_if_exists() {
    let c = context(); // anonymous: no aws:username
    let equals = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringEquals":{"aws:username":"ann"}}}"#,
    );
    assert_eq!(
        ask(&equals, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
    let not_equals = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringNotEquals":{"aws:username":"ann"}}}"#,
    );
    assert!(matches!(
        ask(
            &not_equals,
            Requester::Anonymous,
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::Allow { .. }
    ));
    let if_exists = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringEqualsIfExists":{"aws:username":"ann"}}}"#,
    );
    assert!(matches!(
        ask(
            &if_exists,
            Requester::Anonymous,
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::Allow { .. }
    ));
    let acme = space();
    assert_eq!(
        ask(
            &if_exists,
            Requester::User {
                space: &acme,
                name: "bob"
            },
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::NotApplicable
    );
    let null = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"Null":{"aws:username":"true"}}}"#,
    );
    assert!(matches!(
        ask(&null, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Allow { .. }
    ));
    assert_eq!(
        ask(&null, ann(&acme), "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn a_negated_operator_with_several_values_matches_only_when_none_equals() {
    let policy = policy(
        r#"{"Effect":"Deny","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringNotEquals":{"aws:username":["ann","bob"]}}}"#,
    );
    let acme = space();
    let c = context();
    assert_eq!(
        ask(&policy, ann(&acme), "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
    assert!(matches!(
        ask(
            &policy,
            Requester::User {
                space: &acme,
                name: "eve"
            },
            "s3:GetObject",
            Some("k"),
            &c
        ),
        PolicyAnswer::Deny { .. }
    ));
}

#[test]
fn every_condition_of_a_statement_must_hold() {
    let policy = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"Bool":{"aws:SecureTransport":"true"},"IpAddress":{"aws:SourceIp":"203.0.113.0/24"}}}"#,
    );
    let mut c = context();
    assert!(matches!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Allow { .. }
    ));
    c.secure_transport = false;
    assert_eq!(
        ask(&policy, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
}

#[test]
fn date_numeric_and_like_operators_compare_as_their_types() {
    let c = context(); // now 1_800_000_000 = 2027-01-15T08:00:00Z
    let before = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"DateLessThan":{"aws:CurrentTime":"2027-01-16T00:00:00Z"}}}"#,
    );
    assert!(matches!(
        ask(&before, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Allow { .. }
    ));
    let after = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"DateGreaterThan":{"aws:EpochTime":"1800000001"}}}"#,
    );
    assert_eq!(
        ask(&after, Requester::Anonymous, "s3:GetObject", Some("k"), &c),
        PolicyAnswer::NotApplicable
    );
    let mut listing = context();
    listing.max_keys = Some(10);
    listing.prefix = Some("home/ann/".to_owned());
    let bounded = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:ListBucket","Resource":"arn:aws:s3:::photos","Condition":{"NumericLessThanEquals":{"s3:max-keys":"10"},"StringLike":{"s3:prefix":"home/*"}}}"#,
    );
    assert!(matches!(
        ask(
            &bounded,
            Requester::Anonymous,
            "s3:ListBucket",
            None,
            &listing
        ),
        PolicyAnswer::Allow { .. }
    ));
    listing.max_keys = Some(11);
    assert_eq!(
        ask(
            &bounded,
            Requester::Anonymous,
            "s3:ListBucket",
            None,
            &listing
        ),
        PolicyAnswer::NotApplicable
    );
    let ignore_case = policy(
        r#"{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringEqualsIgnoreCase":{"aws:username":"ANN"}}}"#,
    );
    let acme = space();
    assert!(matches!(
        ask(&ignore_case, ann(&acme), "s3:GetObject", Some("k"), &c),
        PolicyAnswer::Allow { .. }
    ));
}
