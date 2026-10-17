//! Writing a bucket policy: what is accepted, and every shape refused before it can be stored.

use super::super::model::SpaceName;
use super::{BucketPolicy, PolicyError};

fn space() -> SpaceName {
    SpaceName::new("acme").expect("space")
}

fn parse(text: &str) -> Result<BucketPolicy, PolicyError> {
    BucketPolicy::parse(text, "photos", &space())
}

fn refused(text: &str) -> String {
    parse(text).expect_err("refused").to_string()
}

const PUBLIC_READ: &str = r#"{"Version":"2012-10-17","Statement":[{"Sid":"read","Effect":"Allow","Principal":"*",
"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}]}"#;

#[test]
fn a_public_read_policy_is_accepted() {
    let policy = parse(PUBLIC_READ).expect("accepted");
    assert_eq!(policy.statements().len(), 1);
}

#[test]
fn a_single_statement_object_and_a_principal_object_are_accepted() {
    let text = r#"{"Version":"2008-10-17","Statement":{"Effect":"Deny","Principal":{"AWS":["arn:aws:iam::acme:user/ann","arn:aws:iam::acme:root"]},
    "Action":["s3:Delete*"],"Resource":["arn:aws:s3:::photos","arn:aws:s3:::photos/*"],
    "Condition":{"Bool":{"aws:SecureTransport":"false"}}}}"#;
    assert_eq!(parse(text).expect("accepted").statements().len(), 1);
}

#[test]
fn every_malformed_shape_is_refused_naming_its_fault() {
    let cases = [
        ("not json", "not JSON"),
        (r#"{"Statement":[]}"#, "Version"),
        (r#"{"Version":"2020-01-01","Statement":[]}"#, "Version"),
        (r#"{"Version":"2012-10-17","Statement":[]}"#, "no statement"),
        (
            r#"{"Version":"2012-10-17","Statement":[],"Id":"x","Extra":1}"#,
            "Extra",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Maybe","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "Effect",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "Principal",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","NotPrincipal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "NotPrincipal",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObjekt","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "s3:GetObjekt",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"ec2:RunInstances","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "ec2:RunInstances",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::other/*"}]}"#,
            "arn:aws:s3:::other/*",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/${aws:username}"}]}"#,
            "variable",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"AWS":"arn:aws:iam::other:user/ann"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "arn:aws:iam::other:user/ann",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","NotAction":"s3:PutObject","Resource":"arn:aws:s3:::photos/*"}]}"#,
            "NotAction",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringMaybe":{"aws:username":"ann"}}}]}"#,
            "StringMaybe",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"StringEquals":{"aws:PrincipalOrgID":"o-1"}}}]}"#,
            "aws:PrincipalOrgID",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"ForAnyValue:StringEquals":{"aws:username":"ann"}}}]}"#,
            "ForAnyValue",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"IpAddress":{"aws:SourceIp":"10.0.0.0/33"}}}]}"#,
            "10.0.0.0/33",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"DateLessThan":{"aws:CurrentTime":"soon"}}}]}"#,
            "soon",
        ),
        (
            r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos/*","Condition":{"Null":{"aws:username":"maybe"}}}]}"#,
            "maybe",
        ),
    ];
    for (text, named) in cases {
        let message = refused(text);
        assert!(message.contains(named), "{text}\n→ {message}");
    }
}

#[test]
fn a_policy_over_twenty_kilobytes_is_refused() {
    let padding = " ".repeat(20 * 1024);
    let text = format!("{PUBLIC_READ}{padding}");
    assert!(refused(&text).contains("20"));
}
