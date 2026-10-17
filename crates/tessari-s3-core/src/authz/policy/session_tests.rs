//! A session policy: no principal, any S3 resource, at most 2 048 characters, and evaluated with deny overriding allow
//! over the resource a request names — a bucket and key, a bucket, or `arn:aws:s3:::*` for an action naming none.

use super::super::model::SpaceName;
use super::{Context, PolicyAnswer, PolicyRequest, Requester, SessionPolicy};

fn session(statements: &str) -> SessionPolicy {
    let text = format!(r#"{{"Version":"2012-10-17","Statement":[{statements}]}}"#);
    SessionPolicy::parse(&text).expect("accepted")
}

fn ask(
    policy: &SessionPolicy,
    action: &str,
    bucket: Option<&str>,
    key: Option<&str>,
) -> PolicyAnswer {
    let space = SpaceName::new("acme").expect("space");
    let context = Context::default();
    policy.evaluate(
        &PolicyRequest {
            requester: Requester::User {
                space: &space,
                name: "ann",
            },
            action,
            key,
            context: &context,
        },
        bucket,
    )
}

#[test]
fn an_allow_covers_only_the_actions_and_resources_it_names_and_a_deny_outweighs_it() {
    let policy = session(
        r#"{"Effect":"Allow","Action":["s3:GetObject","s3:ListBucket"],"Resource":["arn:aws:s3:::photos","arn:aws:s3:::photos/*"]},
           {"Sid":"secret","Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/private/*"}"#,
    );
    assert!(matches!(
        ask(&policy, "s3:GetObject", Some("photos"), Some("a.png")),
        PolicyAnswer::Allow { .. }
    ));
    assert!(matches!(
        ask(&policy, "s3:ListBucket", Some("photos"), None),
        PolicyAnswer::Allow { .. }
    ));
    assert_eq!(
        ask(&policy, "s3:GetObject", Some("photos"), Some("private/k")),
        PolicyAnswer::Deny {
            sid: Some("secret".to_owned())
        }
    );
    for (action, bucket, key) in [
        ("s3:PutObject", Some("photos"), Some("a.png")),
        ("s3:GetObject", Some("other"), Some("a.png")),
        ("s3:ListAllMyBuckets", None, None),
    ] {
        assert_eq!(
            ask(&policy, action, bucket, key),
            PolicyAnswer::NotApplicable,
            "{action} {bucket:?}"
        );
    }
    let everything = session(r#"{"Effect":"Allow","Action":"s3:*","Resource":"*"}"#);
    assert!(matches!(
        ask(&everything, "s3:ListAllMyBuckets", None, None),
        PolicyAnswer::Allow { .. }
    ));
}

#[test]
fn a_document_a_session_policy_cannot_be_is_refused_naming_why() {
    let long = format!(
        r#"{{"Version":"2012-10-17","Statement":[{{"Sid":"{}","Effect":"Allow","Action":"s3:*","Resource":"*"}}]}}"#,
        "a".repeat(2048)
    );
    for (text, why) in [
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:*","Resource":"*"}]}"#,
            "no Principal",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"arn:aws:iam::acme:user/ann"}]}"#,
            "not an S3 resource",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"iam:*","Resource":"*"}]}"#,
            "not an S3 action",
        ),
        (r#"{"Statement":[]}"#, "Version"),
        (long.as_str(), "at most 2048 characters"),
    ] {
        let refused = SessionPolicy::parse(text).expect_err(why);
        assert!(refused.to_string().contains(why), "{refused} lacks {why}");
    }
}
